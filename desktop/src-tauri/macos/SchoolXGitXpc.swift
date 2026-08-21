import Darwin
import Dispatch
import Foundation
import XPC


enum ClientCompletion {
  case pending
  case finished(Int32)
  case failed(String)
}

private enum AuthorityFailureDisposition {
  case startCleanup
  case waitForServiceCleanup
  case cleanupProven
  case alreadyCleaning
  case terminal
}

private enum ChildCleanupDisposition {
  case clean
  case pending
  case unknowable
}

final class ClientOperation {
  let requestID: UInt64
  let session: ClientAuthoritySession
  let connection: xpc_connection_t
  let queue: DispatchQueue
  let slotFD: Int32
  let slot: UnsafeMutablePointer<Int32>
  let slotLength: Int
  let nonceHigh: UInt64
  let nonceLow: UInt64
  let lock = NSLock()
  var pid: Int32 = 0
  var helperPID: Int32 = 0
  var helperPGID: Int32 = 0
  var helperSID: Int32 = 0
  var launchSent = false
  var cleanupPGID: Int32 = 0
  var childTerminationAttempted = false
  var deferredAbortCleanup = false
  var helperExitProven = false
  var authorityRetentionRequired = false
  var authorityCleanupStarted = false
  var clientAbortOwnerClaimed = false
  var noChildCouldHaveSpawned = false
  var waitingForServiceCleanup = false
  var serviceQuarantined = false
  var launchCallActive = true
  var clientHandleExposed = false
  var removeAfterFinishedAcknowledgment = false
  var clientRemovalClaimed = false
  var helperExitTimer: DispatchSourceTimer?
  var serviceProofTimer: DispatchSourceTimer?
  var completion = ClientCompletion.pending

  init(
    requestID: UInt64,
    session: ClientAuthoritySession,
    slotFD: Int32,
    slot: UnsafeMutablePointer<Int32>,
    slotLength: Int,
    nonceHigh: UInt64,
    nonceLow: UInt64
  ) {
    self.requestID = requestID
    self.session = session
    self.connection = session.connection
    self.queue = session.queue
    self.slotFD = slotFD
    self.slot = slot
    self.slotLength = slotLength
    self.nonceHigh = nonceHigh
    self.nonceLow = nonceLow
    if let helper = session.helperIdentity() {
      helperPID = helper.0
      helperPGID = helper.1
      helperSID = helper.2
    }
  }

  deinit {
    helperExitTimer?.cancel()
    serviceProofTimer?.cancel()
    _ = munmap(UnsafeMutableRawPointer(slot), slotLength)
    _ = close(slotFD)
  }

  func installPID(_ value: Int32) -> Bool {
    lock.lock()
    defer { lock.unlock() }
    guard value > 1, readPIDSlot(fd: slotFD) == value,
      pid == 0 || pid == value,
      helperSID > 0,
      getpgid(value) == value,
      getsid(value) == helperSID
    else {
      return false
    }
    pid = value
    return true
  }

  func matchesHelper(pid: Int32, sid: Int32) -> Bool {
    lock.lock()
    defer { lock.unlock() }
    return helperPID == pid && helperPGID == pid && helperSID == sid
  }

  func markLaunchSent() -> Bool {
    lock.lock()
    defer { lock.unlock() }
    guard case .pending = completion, !clientRemovalClaimed, !authorityCleanupStarted else {
      return false
    }
    launchSent = true
    return true
  }

  func quarantineService(_ diagnostic: String, permanent: Bool) {
    lock.lock()
    guard case .pending = completion, !clientRemovalClaimed else {
      lock.unlock()
      return
    }
    completion = .failed(diagnostic)
    authorityRetentionRequired = true
    authorityCleanupStarted = true
    var observeServiceCleanup = false
    var cleanupAlreadyProven = false
    switch loadPIDSlotOwner(slot) {
    case .clientAborting:
      clientAbortOwnerClaimed = true
    case .cleanupProven:
      authorityRetentionRequired = false
      cleanupAlreadyProven = true
    case .serviceCleaning:
      waitingForServiceCleanup = true
      serviceQuarantined = permanent
      observeServiceCleanup = true
    default:
      serviceQuarantined = true
    }
    lock.unlock()
    if cleanupAlreadyProven {
      autoRemoveAfterAuthorityRelease()
    } else if observeServiceCleanup {
      armServiceProofObservation()
    }
  }

  func finish(rawStatus: Int32) {
    lock.lock()
    if case .pending = completion {
      completion = .finished(rawStatus)
    }
    lock.unlock()
  }

  func fail(_ diagnostic: String) {
    switch beginAuthorityFailure(diagnostic) {
    case .startCleanup:
      DispatchQueue.global(qos: .userInitiated).async { [weak self] in
        _ = self?.performAuthorityCleanup()
      }
    case .cleanupProven:
      autoRemoveAfterAuthorityRelease()
    case .waitForServiceCleanup:
      armServiceProofObservation()
    case .alreadyCleaning, .terminal:
      break
    }
  }

  func finishWithFailure(_ diagnostic: String) {
    _ = recordFailure(diagnostic)
  }

  func acceptServiceFinished(rawStatus: Int32, error: String?) -> Bool {
    lock.lock()
    guard loadPIDSlotOwner(slot) == .cleanupProven else {
      lock.unlock()
      return false
    }
    authorityRetentionRequired = false
    waitingForServiceCleanup = false
    serviceQuarantined = false
    let timer = serviceProofTimer
    serviceProofTimer = nil
    switch completion {
    case .pending:
      completion = error.map(ClientCompletion.failed) ?? .finished(rawStatus)
    case .failed:
      removeAfterFinishedAcknowledgment = !clientHandleExposed
    case .finished:
      lock.unlock()
      timer?.cancel()
      return false
    }
    lock.unlock()
    timer?.cancel()
    return true
  }

  func acceptServiceCleanupProof() -> Bool {
    lock.lock()
    guard loadPIDSlotOwner(slot) == .cleanupProven else {
      lock.unlock()
      return false
    }
    authorityRetentionRequired = false
    waitingForServiceCleanup = false
    serviceQuarantined = false
    let timer = serviceProofTimer
    serviceProofTimer = nil
    lock.unlock()
    timer?.cancel()
    return true
  }

  func shouldRemoveAfterServiceFinished() -> Bool {
    lock.lock()
    defer { lock.unlock() }
    return !authorityRetentionRequired && removeAfterFinishedAcknowledgment
      && !launchCallActive && !clientHandleExposed
  }

  func finishLaunchWithExposedHandle() -> Bool {
    lock.lock()
    defer { lock.unlock() }
    guard launchCallActive, !clientRemovalClaimed else { return false }
    switch completion {
    case .pending, .finished:
      launchCallActive = false
      clientHandleExposed = true
      return true
    case .failed:
      return false
    }
  }

  func finishLaunchWithoutExposedHandle() {
    lock.lock()
    launchCallActive = false
    lock.unlock()
  }

  func retireExposedHandle() {
    lock.lock()
    clientHandleExposed = false
    lock.unlock()
  }

  func recoverAfterCancelFailureIfHelperExited() {
    lock.lock()
    let child = pid
    let helper = helperPID
    let helperGone = helperExitProven
    let owner = loadPIDSlotOwner(slot)
    lock.unlock()
    if owner == .cleanupProven {
      _ = acceptServiceCleanupProof()
      autoRemoveAfterAuthorityRelease()
      return
    }
    guard child > 1, helper > 1, owner == .serviceCleaning else { return }
    if helperGone {
      takeoverOrphanedServiceCleanup()
      return
    }
    errno = 0
    if kill(helper, 0) != 0, errno == ESRCH {
      authenticatedHelperDidExit()
    }
  }

  @discardableResult
  func abortLaunch(_ diagnostic: String) -> Bool {
    switch beginAuthorityFailure(diagnostic) {
    case .startCleanup:
      return performAuthorityCleanup()
    case .cleanupProven:
      return true
    case .waitForServiceCleanup:
      armServiceProofObservation()
      return false
    case .alreadyCleaning:
      return false
    case .terminal:
      return true
    }
  }

  private func beginAuthorityFailure(_ diagnostic: String) -> AuthorityFailureDisposition {
    lock.lock()
    defer { lock.unlock() }
    guard !clientRemovalClaimed else { return .terminal }
    if serviceQuarantined { return .alreadyCleaning }
    switch completion {
    case .pending:
      completion = .failed(diagnostic)
      authorityRetentionRequired = true
    case .failed where authorityRetentionRequired:
      break
    case .finished, .failed:
      return .terminal
    }
    guard !authorityCleanupStarted else { return .alreadyCleaning }
    authorityCleanupStarted = true
    switch claimClientAbortOwner(slot) {
    case .acquired(let noChild):
      clientAbortOwnerClaimed = true
      noChildCouldHaveSpawned = noChild
      if noChild {
        guard proveClientCleanupOwner(slot) else {
          serviceQuarantined = true
          return .alreadyCleaning
        }
        authorityRetentionRequired = false
        return .cleanupProven
      }
      return .startCleanup
    case .alreadyOwned:
      clientAbortOwnerClaimed = true
      return .startCleanup
    case .serviceCleaning:
      waitingForServiceCleanup = true
      return .waitForServiceCleanup
    case .cleanupProven:
      authorityRetentionRequired = false
      return .cleanupProven
    case .invalid:
      serviceQuarantined = true
      return .alreadyCleaning
    }
  }

  private func armServiceProofObservation() {
    let timer = DispatchSource.makeTimerSource(queue: queue)
    timer.schedule(deadline: .now() + .milliseconds(100), repeating: .milliseconds(100))
    timer.setEventHandler { [weak self] in
      self?.observeServiceCleanupProof()
    }
    lock.lock()
    guard waitingForServiceCleanup, serviceProofTimer == nil else {
      lock.unlock()
      timer.resume()
      timer.cancel()
      return
    }
    serviceProofTimer = timer
    lock.unlock()
    timer.resume()
  }

  private func observeServiceCleanupProof() {
    lock.lock()
    guard waitingForServiceCleanup else {
      lock.unlock()
      return
    }
    let owner = loadPIDSlotOwner(slot)
    let helper = helperPID
    let helperGone = helperExitProven
    if case .cleanupProven = owner {
      waitingForServiceCleanup = false
      authorityRetentionRequired = false
      serviceQuarantined = false
      let timer = serviceProofTimer
      serviceProofTimer = nil
      lock.unlock()
      timer?.cancel()
      autoRemoveAfterAuthorityRelease()
      return
    }
    lock.unlock()
    if helperGone {
      takeoverOrphanedServiceCleanup()
      return
    }
    guard helper > 1 else { return }
    errno = 0
    if kill(helper, 0) != 0, errno == ESRCH { authenticatedHelperDidExit() }
  }

  func authenticatedSessionHelperDidExit() {
    authenticatedHelperDidExit()
  }

  private func authenticatedHelperDidExit() {
    lock.lock()
    helperExitProven = true
    let serviceWasCleaning = waitingForServiceCleanup
    let clientWasCleaning = clientAbortOwnerClaimed && authorityCleanupStarted
    lock.unlock()
    if serviceWasCleaning { takeoverOrphanedServiceCleanup() }
    if clientWasCleaning {
      rememberChildAuthority(readPIDSlot(fd: slotFD))
      _ = performAuthorityCleanup()
    }
  }

  private func takeoverOrphanedServiceCleanup() {
    lock.lock()
    guard waitingForServiceCleanup, helperExitProven else {
      lock.unlock()
      return
    }
    switch claimOrphanedServiceCleanupOwner(slot) {
    case .acquired:
      clientAbortOwnerClaimed = true
      waitingForServiceCleanup = false
      serviceQuarantined = false
    case .cleanupProven:
      waitingForServiceCleanup = false
      authorityRetentionRequired = false
      serviceQuarantined = false
      let timer = serviceProofTimer
      serviceProofTimer = nil
      lock.unlock()
      timer?.cancel()
      autoRemoveAfterAuthorityRelease()
      return
    case .alreadyOwned, .serviceCleaning, .invalid:
      lock.unlock()
      return
    }
    let timer = serviceProofTimer
    serviceProofTimer = nil
    lock.unlock()
    timer?.cancel()
    guard let observed = readPIDSlotExact(fd: slotFD) else {
      lock.lock()
      serviceQuarantined = true
      lock.unlock()
      return
    }
    lock.lock()
    let cachedPID = pid
    lock.unlock()
    if cachedPID <= 1 && observed == 0 {
      lock.lock()
      guard proveClientCleanupOwner(slot) else {
        serviceQuarantined = true
        lock.unlock()
        return
      }
      authorityRetentionRequired = false
      lock.unlock()
      autoRemoveAfterAuthorityRelease()
      return
    }
    let childAuthority: Int32
    if cachedPID > 1 {
      guard observed == -1 || observed == cachedPID else {
        lock.lock()
        serviceQuarantined = true
        lock.unlock()
        return
      }
      childAuthority = cachedPID
    } else if observed > 1 {
      childAuthority = observed
    } else {
      lock.lock()
      serviceQuarantined = true
      lock.unlock()
      return
    }
    lock.lock()
    cleanupPGID = childAuthority
    childTerminationAttempted = true
    lock.unlock()
    _ = finishAuthorityCleanupAfterHelperExit(helper: helperPID)
  }

  private func performAuthorityCleanup() -> Bool {
    lock.lock()
    let ownsAbort = clientAbortOwnerClaimed && !noChildCouldHaveSpawned
    let helperGone = helperExitProven
    let helper = helperPID
    lock.unlock()
    guard ownsAbort, helperGone,
      loadPIDSlotOwner(slot) == .clientAborting
    else { return false }
    rememberChildAuthority(readPIDSlot(fd: slotFD))
    lock.lock()
    childTerminationAttempted = true
    lock.unlock()
    return finishAuthorityCleanupAfterHelperExit(helper: helper)
  }

  func claimClientRemoval() -> Bool {
    lock.lock()
    defer { lock.unlock() }
    guard !authorityRetentionRequired, !clientRemovalClaimed else { return false }
    clientRemovalClaimed = true
    return true
  }

  private func rememberChildAuthority(_ observed: Int32) {
    lock.lock()
    defer { lock.unlock() }
    guard cleanupPGID == 0 else { return }
    if pid > 1 {
      cleanupPGID = pid
    } else if observed > 1 {
      cleanupPGID = observed
    }
  }

  private func childCleanupDisposition() -> ChildCleanupDisposition {
    lock.lock()
    let child = cleanupPGID
    let mayHaveLaunched = launchSent
    lock.unlock()
    guard child > 1 else { return mayHaveLaunched ? .unknowable : .clean }
    errno = 0
    if kill(-child, 0) != 0, errno == ESRCH { return .clean }
    return .pending
  }

  private func finishAuthorityCleanupAfterHelperExit(helper: Int32) -> Bool {
    switch childCleanupDisposition() {
    case .clean:
      lock.lock()
      guard clientAbortOwnerClaimed, proveClientCleanupOwner(slot) else {
        serviceQuarantined = true
        lock.unlock()
        return false
      }
      authorityRetentionRequired = false
      lock.unlock()
      autoRemoveAfterAuthorityRelease()
      return true
    case .pending:
      guard helper > 1 else { return false }
      armDeferredAbortCleanup(helper: helper)
      return false
    case .unknowable:
      // A launch was sent but neither the authenticated Started reply nor the
      // nonce-bound PID slot preserved a PGID. Releasing authority would make
      // an unproven child overlap a later mutation, so quarantine this client
      // operation for the remainder of the helper lifetime.
      return false
    }
  }

  private func armDeferredAbortCleanup(helper: Int32) {
    let timer = DispatchSource.makeTimerSource(queue: queue)
    timer.schedule(deadline: .now() + .milliseconds(100), repeating: .milliseconds(100))
    timer.setEventHandler { [weak self] in
      self?.pollDeferredAbortCleanup(helper: helper)
    }
    lock.lock()
    guard !deferredAbortCleanup else {
      lock.unlock()
      timer.resume()
      timer.cancel()
      return
    }
    deferredAbortCleanup = true
    let helperGone = helperExitProven
    helperExitTimer = timer
    lock.unlock()
    timer.resume()
    if helperGone {
      queue.async { [weak self] in self?.authenticatedHelperDidExit() }
      return
    }
    errno = 0
    if kill(helper, 0) != 0, errno == ESRCH {
      queue.async { [weak self] in self?.authenticatedHelperDidExit() }
    }
  }

  private func pollDeferredAbortCleanup(helper: Int32) {
    lock.lock()
    let active = deferredAbortCleanup
    let helperGone = helperExitProven
    lock.unlock()
    guard active else { return }
    if !helperGone {
      errno = 0
      guard kill(helper, 0) != 0, errno == ESRCH else { return }
      authenticatedHelperDidExit()
      return
    }
    continueDeferredAbortCleanup()
  }

  private func continueDeferredAbortCleanup() {
    switch childCleanupDisposition() {
    case .pending:
      return
    case .clean:
      lock.lock()
      guard deferredAbortCleanup, helperExitProven else {
        lock.unlock()
        return
      }
      guard clientAbortOwnerClaimed, proveClientCleanupOwner(slot) else {
        serviceQuarantined = true
        deferredAbortCleanup = false
        let timer = helperExitTimer
        helperExitTimer = nil
        lock.unlock()
        timer?.cancel()
        return
      }
      deferredAbortCleanup = false
      authorityRetentionRequired = false
      let timer = helperExitTimer
      helperExitTimer = nil
      lock.unlock()
      timer?.cancel()
      autoRemoveAfterAuthorityRelease()
    case .unknowable:
      lock.lock()
      guard deferredAbortCleanup, helperExitProven else {
        lock.unlock()
        return
      }
      deferredAbortCleanup = false
      let timer = helperExitTimer
      helperExitTimer = nil
      lock.unlock()
      timer?.cancel()
      // Keep authorityRetentionRequired set: no safe PGID cleanup authority
      // survived the helper failure, so later Git mutations remain blocked.
    }
  }

  @discardableResult
  private func recordFailure(_ diagnostic: String) -> Bool {
    lock.lock()
    var installed = false
    if case .pending = completion {
      completion = .failed(diagnostic)
      installed = true
    }
    lock.unlock()
    return installed
  }

  private func autoRemoveAfterAuthorityRelease() {
    lock.lock()
    let shouldRemove = !authorityRetentionRequired && !launchCallActive
      && !clientHandleExposed
    lock.unlock()
    if shouldRemove { removeClientOperation(requestID, cancel: true) }
  }

  func poll() -> ClientCompletion {
    lock.lock()
    defer { lock.unlock() }
    return completion
  }

  func childAuthorityDisposition() -> (proven: Bool, retained: Bool) {
    lock.lock()
    defer { lock.unlock() }
    let proven = !authorityRetentionRequired && loadPIDSlotOwner(slot) == .cleanupProven
    return (proven, !proven)
  }
}

private let clientOperationsLock = NSLock()
private var clientOperations: [UInt64: ClientOperation] = [:]

func schoolx_git_xpc_is_service() -> Bool {
  Bundle.main.bundleIdentifier == schoolXGitServiceIdentifier
    && Bundle.main.bundleURL.pathExtension == "xpc"
}

func schoolx_git_xpc_capability() -> RustString {
  RustString(capabilityDiagnostic() ?? "")
}

func schoolx_git_xpc_launch(
  session_id: UInt64,
  request_id: UInt64,
  family: UInt8,
  cwd_fd: Int32,
  stdin_fd: Int32,
  stdout_fd: Int32,
  stderr_fd: Int32,
  payload: RustString
) -> RustString {
  guard let session = lookupClientSession(session_id) else {
    return encodeChildFailure(
      "unknown XPC authority session",
      cleanupProvenWithoutOperation: false
    )
  }
  if let diagnostic = capabilityDiagnostic() {
    return encodeChildFailure(diagnostic)
  }
  guard request_id != 0 else {
    return encodeChildFailure("invalid XPC request identifier")
  }
  let payload = payload.toString()
  guard payload.utf8.count <= maximumPayloadBytes else {
    return encodeChildFailure("typed Git XPC payload exceeded its limit")
  }
  for (descriptor, label) in [
    (cwd_fd, "cwd"), (stdin_fd, "stdin"), (stdout_fd, "stdout"),
    (stderr_fd, "stderr"),
  ] where descriptor < 0 {
    return encodeChildFailure("invalid XPC Git \(label) descriptor")
  }

  let nonceHigh = randomNonceComponent()
  let nonceLow = randomNonceComponent()
  let slotResult = createPIDSlot(nonceHigh: nonceHigh, nonceLow: nonceLow)
  guard case .success(let slotFD, let slot, let slotLength) = slotResult else {
    if case .failure(let diagnostic) = slotResult {
      return encodeChildFailure(diagnostic)
    }
    return encodeChildFailure("failed to create XPC PID slot")
  }
  guard session.reserveChild(request_id) else {
    _ = munmap(UnsafeMutableRawPointer(slot), slotLength)
    _ = close(slotFD)
    return encodeChildFailure("XPC authority session already has a child")
  }
  let operation = ClientOperation(
    requestID: request_id,
    session: session,
    slotFD: slotFD,
    slot: slot,
    slotLength: slotLength,
    nonceHigh: nonceHigh,
    nonceLow: nonceLow
  )
  guard installClientOperation(operation) else {
    session.releaseChild(operation)
    return encodeChildFailure("duplicate XPC Git request identifier")
  }
  var exposedHandle = false
  defer {
    if !exposedHandle {
      operation.finishLaunchWithoutExposedHandle()
      removeClientOperation(request_id, cancel: true)
    }
  }

  let launch = xpc_dictionary_create(nil, nil, 0)
  xpc_dictionary_set_string(launch, "kind", "launch")
  setSessionEnvelope(launch, operation: operation)
  xpc_dictionary_set_uint64(launch, "requestId", request_id)
  xpc_dictionary_set_uint64(launch, "nonceHigh", nonceHigh)
  xpc_dictionary_set_uint64(launch, "nonceLow", nonceLow)
  xpc_dictionary_set_uint64(launch, "family", UInt64(family))
  xpc_dictionary_set_string(launch, "payload", payload)
  xpc_dictionary_set_fd(launch, "cwdFd", cwd_fd)
  xpc_dictionary_set_fd(launch, "stdinFd", stdin_fd)
  xpc_dictionary_set_fd(launch, "stdoutFd", stdout_fd)
  xpc_dictionary_set_fd(launch, "stderrFd", stderr_fd)
  xpc_dictionary_set_fd(launch, "pidSlotFd", slotFD)

  guard operation.markLaunchSent() else {
    return encodeChildFailure("signed XPC helper connection failed before launch", operation)
  }
  let launchReply = ReplyBox()
  xpc_connection_send_message_with_reply(operation.connection, launch, operation.queue) { reply in
    launchReply.store(reply)
  }
  guard launchReply.wait(timeout: launchTimeout), let reply = launchReply.take() else {
    if operation.abortLaunch("signed XPC helper launch timed out") {
      removeClientOperation(request_id, cancel: true)
    }
    return encodeChildFailure("signed XPC helper launch timed out", operation)
  }
  if let diagnostic = parseLaunchRejection(reply, operation: operation) {
    _ = operation.abortLaunch(diagnostic)
    removeClientOperation(request_id, cancel: true)
    return encodeChildFailure(diagnostic, operation)
  }
  if let diagnostic = parseServiceQuarantine(
    reply,
    kind: "launchQuarantined",
    operation: operation
  ) {
    operation.quarantineService(diagnostic, permanent: false)
    removeClientOperation(request_id, cancel: true)
    return encodeChildFailure(diagnostic, operation)
  }
  guard let started = parseStartedReply(reply, operation: operation) else {
    if operation.abortLaunch("signed XPC helper returned an invalid launch reply") {
      removeClientOperation(request_id, cancel: true)
    }
    return encodeChildFailure("signed XPC helper returned an invalid launch reply", operation)
  }

  let resume = xpc_dictionary_create(nil, nil, 0)
  xpc_dictionary_set_string(resume, "kind", "resume")
  setSessionEnvelope(resume, operation: operation)
  xpc_dictionary_set_uint64(resume, "requestId", request_id)
  xpc_dictionary_set_uint64(resume, "nonceHigh", nonceHigh)
  xpc_dictionary_set_uint64(resume, "nonceLow", nonceLow)
  let resumeReply = ReplyBox()
  xpc_connection_send_message_with_reply(operation.connection, resume, operation.queue) { reply in
    resumeReply.store(reply)
  }
  guard resumeReply.wait(timeout: resumeTimeout), let resumed = resumeReply.take(),
    xpc_get_type(resumed) == XPC_TYPE_DICTIONARY,
    dictionaryString(resumed, key: "kind") == "resumed",
    xpc_dictionary_get_uint64(resumed, "requestId") == request_id,
    xpc_dictionary_get_uint64(resumed, "nonceHigh") == nonceHigh,
    xpc_dictionary_get_uint64(resumed, "nonceLow") == nonceLow,
    messageMatchesSession(resumed, operation: operation)
  else {
    let diagnostic = "signed XPC helper did not resume Git"
    if let cancelError = requestServiceCancellation(operation, requestID: request_id) {
      _ = operation.abortLaunch("\(diagnostic): \(cancelError)")
      operation.recoverAfterCancelFailureIfHelperExited()
    } else {
      operation.finishWithFailure(diagnostic)
    }
    return encodeChildFailure(diagnostic, operation)
  }
  guard operation.finishLaunchWithExposedHandle() else {
    return encodeChildFailure("signed XPC helper failed before exposing Git", operation)
  }
  exposedHandle = true
  return encodeRustString(
    EncodedChildResult(
      ok: true,
      pid: UInt32(started),
      childCleanupProven: false,
      childAuthorityRetained: true
    ))
}

private func installClientOperation(_ operation: ClientOperation) -> Bool {
  clientOperationsLock.lock()
  defer { clientOperationsLock.unlock() }
  guard clientOperations.isEmpty, clientOperations[operation.requestID] == nil else { return false }
  clientOperations[operation.requestID] = operation
  return true
}

func lookupClientOperation(_ requestID: UInt64) -> ClientOperation? {
  clientOperationsLock.lock()
  defer { clientOperationsLock.unlock() }
  return clientOperations[requestID]
}

func removeClientOperation(_ requestID: UInt64, cancel: Bool) {
  clientOperationsLock.lock()
  let operation = clientOperations[requestID]
  let mayRemove = operation?.claimClientRemoval() ?? false
  let removed = mayRemove ? clientOperations.removeValue(forKey: requestID) : nil
  clientOperationsLock.unlock()
  if let removed {
    removed.session.releaseChild(removed)
  }
}
