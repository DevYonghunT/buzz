import Darwin
import Dispatch
import Foundation
import XPC

enum ServiceCancelDisposition {
  case kill(Int32)
  case awaitCleanup
  case clientTakeover
  case completed(String?)
}

enum ServiceCleanupDisposition {
  case kill(Int32)
  case awaitCleanup
  case clientTakeover
  case cleanupProven
  case noAuthority
  case invalid
}

enum TerminalChildResetDisposition {
  case success(xpc_object_t?)
  case failure
}

let serviceConnectionsLock = NSLock()
var serviceConnections: [UUID: ServiceConnectionState] = [:]
let activeServiceOperationLock = NSLock()
var activeServiceOperation: UUID?

final class ServiceConnectionState {
  let identity = UUID()
  let helperIncarnationHigh = randomNonceComponent()
  let helperIncarnationLow = randomNonceComponent()
  let connection: xpc_connection_t
  var clientPID: Int32 = 0
  let lock = NSLock()
  var sessionID: UInt64 = 0
  var sessionNonceHigh: UInt64 = 0
  var sessionNonceLow: UInt64 = 0
  var sessionAdmitted = false
  var sessionAdmissionStarted = false
  var sessionEnding = false
  var sessionCleanupProven = false
  var sessionQuarantined = false
  var gitReservationFD: Int32 = -1
  var childCleanupProven = true
  var lastTerminalRequestID: UInt64 = 0
  var lastTerminalNonceHigh: UInt64 = 0
  var lastTerminalNonceLow: UInt64 = 0
  var lastTerminalError: String?
  var requestID: UInt64 = 0
  var nonceHigh: UInt64 = 0
  var nonceLow: UInt64 = 0
  var pid: Int32 = 0
  var pidSlotFD: Int32 = -1
  var pidSlot: UnsafeMutablePointer<Int32>?
  var pidSlotLength = 0
  var helloed = false
  var launchReserved = false
  var ownerProtocolEngaged = false
  var launched = false
  var resumed = false
  var terminating = false
  var cleanupStarted = false
  var reaperActive = false
  var completed = false
  var serviceQuarantined = false
  var connectionInvalidated = false
  var clientExitProven = false
  var manualTransactionActive = false
  var cleanupError: String?
  var cancelRequest: xpc_object_t?
  var clientExitSource: DispatchSourceProcess?
  var clientCleanupTimer: DispatchSourceTimer?

  init(connection: xpc_connection_t) {
    self.connection = connection
  }

  deinit {
    clientExitSource?.cancel()
    clientCleanupTimer?.cancel()
    if let pidSlot, pidSlotLength > 0 {
      _ = munmap(UnsafeMutableRawPointer(pidSlot), pidSlotLength)
    }
    if pidSlotFD >= 0 { _ = close(pidSlotFD) }
  }

  func armClientExitObservation() -> Bool {
    guard clientPID > 1 else { return false }
    let source = DispatchSource.makeProcessSource(
      identifier: clientPID,
      eventMask: .exit,
      queue: DispatchQueue.global(qos: .userInitiated)
    )
    source.setEventHandler { [weak self] in
      guard let self else { return }
      handleAuthenticatedClientExit(state: self)
    }
    lock.lock()
    guard clientExitSource == nil else {
      lock.unlock()
      source.resume()
      source.cancel()
      return false
    }
    clientExitSource = source
    lock.unlock()
    source.resume()
    return true
  }

  func installSessionHello(
    sessionID: UInt64,
    sessionNonceHigh: UInt64,
    sessionNonceLow: UInt64,
    clientPID: Int32
  ) -> Bool {
    lock.lock()
    defer { lock.unlock() }
    guard !helloed, !launchReserved, !launched, sessionID != 0,
      sessionNonceHigh != 0, sessionNonceLow != 0, clientPID > 1
    else { return false }
    self.sessionID = sessionID
    self.sessionNonceHigh = sessionNonceHigh
    self.sessionNonceLow = sessionNonceLow
    self.clientPID = clientPID
    helloed = true
    return true
  }

  func reserveLaunch(
    sessionID: UInt64,
    sessionNonceHigh: UInt64,
    sessionNonceLow: UInt64,
    requestID: UInt64,
    nonceHigh: UInt64,
    nonceLow: UInt64
  ) -> Bool {
    lock.lock()
    defer { lock.unlock() }
    guard helloed, sessionAdmitted, !sessionEnding, !sessionQuarantined,
      childCleanupProven, !launchReserved, !launched, !completed,
      self.sessionID == sessionID,
      self.sessionNonceHigh == sessionNonceHigh,
      self.sessionNonceLow == sessionNonceLow,
      requestID != 0, nonceHigh != 0, nonceLow != 0
    else { return false }
    self.requestID = requestID
    self.nonceHigh = nonceHigh
    self.nonceLow = nonceLow
    launchReserved = true
    childCleanupProven = false
    return true
  }

  func installPIDSlot(
    fd: Int32,
    slot: UnsafeMutablePointer<Int32>,
    length: Int
  ) -> Bool {
    lock.lock()
    defer { lock.unlock() }
    guard helloed, launchReserved, !ownerProtocolEngaged, !launched, !completed,
      fd >= 0, length >= 8
    else { return false }
    pidSlotFD = fd
    pidSlot = slot
    pidSlotLength = length
    ownerProtocolEngaged = true
    return true
  }

  func claimLaunchOwner() -> ServiceLaunchOwnerClaim {
    lock.lock()
    defer { lock.unlock() }
    guard ownerProtocolEngaged, !launched, !completed, let pidSlot else {
      serviceQuarantined = true
      terminating = true
      return .invalid
    }
    let claim = claimServiceLaunchOwner(pidSlot)
    switch claim {
    case .acquired, .cleanupProven:
      break
    case .clientAborting, .serviceCleaning:
      terminating = true
    case .invalid:
      serviceQuarantined = true
      terminating = true
    }
    return claim
  }

  func installSpawnedPID(_ pid: Int32) -> Bool {
    lock.lock()
    defer { lock.unlock() }
    guard ownerProtocolEngaged, !launched, !completed, pid > 1 else { return false }
    self.pid = pid
    launched = true
    return true
  }

  func sealLaunchOwnerForSpawn() -> Bool {
    lock.lock()
    defer { lock.unlock() }
    guard ownerProtocolEngaged, !launched, !completed, let pidSlot else {
      return false
    }
    return claimServiceRuntimeOwner(pidSlot)
  }

  func currentOwnerState() -> PIDSlotOwnerState? {
    lock.lock()
    defer { lock.unlock() }
    guard let pidSlot else { return nil }
    return loadPIDSlotOwner(pidSlot)
  }

  func markResumed(requestID: UInt64, nonceHigh: UInt64, nonceLow: UInt64) -> Int32? {
    lock.lock()
    defer { lock.unlock() }
    guard launched, !terminating, !cleanupStarted, !completed, !resumed,
      self.requestID == requestID,
      self.nonceHigh == nonceHigh,
      self.nonceLow == nonceLow,
      let pidSlot,
      loadPIDSlotOwner(pidSlot) == .serviceCleaning
    else {
      return nil
    }
    resumed = true
    return pid
  }

  func resumeDeadlineDisposition(
    requestID: UInt64,
    nonceHigh: UInt64,
    nonceLow: UInt64
  ) -> ServiceCleanupDisposition {
    lock.lock()
    defer { lock.unlock() }
    guard launched, !terminating, !cleanupStarted, !completed, !resumed,
      self.requestID == requestID,
      self.nonceHigh == nonceHigh,
      self.nonceLow == nonceLow
    else { return .noAuthority }
    return claimServiceCleanupLocked()
  }

  func beginCancel(
    requestID: UInt64,
    nonceHigh: UInt64,
    nonceLow: UInt64,
    request: xpc_object_t
  ) -> ServiceCancelDisposition? {
    lock.lock()
    defer { lock.unlock() }
    if childCleanupProven, lastTerminalRequestID != 0,
      requestID == lastTerminalRequestID,
      nonceHigh == lastTerminalNonceHigh, nonceLow == lastTerminalNonceLow
    {
      return .completed(lastTerminalError)
    }
    guard launched, self.requestID == requestID,
      self.nonceHigh == nonceHigh,
      self.nonceLow == nonceLow
    else { return nil }
    if completed {
      guard cancelRequest == nil else { return nil }
      cancelRequest = request
      return .awaitCleanup
    }
    guard cancelRequest == nil else { return nil }
    cancelRequest = request
    terminating = true
    if cleanupStarted { return .awaitCleanup }
    switch claimServiceCleanupLocked() {
    case .kill(let pid):
      return .kill(pid)
    case .awaitCleanup:
      return .awaitCleanup
    case .clientTakeover:
      return .clientTakeover
    case .cleanupProven:
      return .completed(cleanupError)
    case .noAuthority, .invalid:
      serviceQuarantined = true
      return .clientTakeover
    }
  }

  func beginCleanup(pid: Int32) -> ServiceCleanupDisposition {
    lock.lock()
    defer { lock.unlock() }
    guard launched, !completed, self.pid == pid else { return .invalid }
    return claimServiceCleanupLocked()
  }

  func beginSpawnFailureCleanup(pid: Int32) -> ServiceCleanupDisposition {
    lock.lock()
    defer { lock.unlock() }
    guard ownerProtocolEngaged, !completed, pid == 0 || self.pid == pid else {
      return .invalid
    }
    return claimServiceCleanupLocked()
  }

  func beginConnectionInvalidationCleanup() -> ServiceCleanupDisposition {
    lock.lock()
    defer { lock.unlock() }
    connectionInvalidated = true
    guard ownerProtocolEngaged else { return .noAuthority }
    if completed { return .cleanupProven }
    if !clientExitProven, clientPID > 1 {
      errno = 0
      clientExitProven = kill(clientPID, 0) != 0 && errno == ESRCH
    }
    return claimServiceCleanupLocked(allowOrphanTakeover: clientExitProven)
  }

  func authenticatedClientDidExit() -> ServiceCleanupDisposition {
    lock.lock()
    defer { lock.unlock() }
    clientExitProven = true
    guard connectionInvalidated, ownerProtocolEngaged else { return .noAuthority }
    if completed { return .cleanupProven }
    return claimServiceCleanupLocked(allowOrphanTakeover: true)
  }

  private func claimServiceCleanupLocked(
    allowOrphanTakeover: Bool = false
  ) -> ServiceCleanupDisposition {
    guard let pidSlot else { return .invalid }
    terminating = true
    let claim = allowOrphanTakeover
      ? claimOrphanedClientCleanupOwner(pidSlot)
      : claimServiceCleanupOwner(pidSlot)
    switch claim {
    case .acquired:
      cleanupStarted = true
      return .kill(pid)
    case .alreadyCleaning:
      if cleanupStarted { return .awaitCleanup }
      cleanupStarted = true
      return .kill(pid)
    case .clientAborting:
      return .clientTakeover
    case .cleanupProven:
      return .cleanupProven
    case .invalid:
      serviceQuarantined = true
      return .invalid
    }
  }

  func markPIDSlotTerminal() -> Bool {
    lock.lock()
    defer { lock.unlock() }
    guard cleanupStarted, !completed, let pidSlot,
      loadPIDSlotOwner(pidSlot) == .serviceCleaning
    else { return false }
    pidSlot.pointee = -1
    return true
  }

  func proveCleanup(error: String?) -> Bool {
    lock.lock()
    defer { lock.unlock() }
    guard ownerProtocolEngaged, cleanupStarted, !completed, let pidSlot,
      proveServiceCleanupOwner(pidSlot)
    else {
      serviceQuarantined = true
      return false
    }
    cleanupStarted = true
    terminating = true
    completed = true
    serviceQuarantined = false
    cleanupError = error
    return true
  }

  func acceptCleanupProof(error: String?) -> Bool {
    lock.lock()
    defer { lock.unlock() }
    guard ownerProtocolEngaged, !completed, let pidSlot,
      let owner = loadPIDSlotOwner(pidSlot)
    else { return false }
    guard case .cleanupProven = owner else { return false }
    cleanupStarted = true
    terminating = true
    completed = true
    serviceQuarantined = false
    cleanupError = error
    return true
  }

  func quarantineServiceAuthority() {
    lock.lock()
    serviceQuarantined = true
    terminating = true
    lock.unlock()
  }

  func mayReleaseConnectionState() -> Bool {
    lock.lock()
    defer { lock.unlock() }
    return !serviceQuarantined && !sessionQuarantined
      && !sessionAdmitted && !sessionEnding && !manualTransactionActive
  }

  func connectionWasInvalidated() -> Bool {
    lock.lock()
    defer { lock.unlock() }
    return connectionInvalidated
  }

}

func schoolx_git_xpc_service_main() -> Int32 {
  guard #available(macOS 12.0, *), configureServiceChildReaping() else {
    return 78
  }
  xpc_main { connection in
    configureServiceConnection(connection)
  }
}

@available(macOS 12.0, *)
private func configureServiceConnection(_ connection: xpc_connection_t) {
  guard let identity = currentSigningIdentity(),
    identity.identifier == schoolXGitServiceIdentifier,
    installPeerRequirement(
      on: connection,
      expectedIdentifier: schoolXAppIdentifier,
      teamIdentifier: identity.teamIdentifier
    ) == nil
  else {
    xpc_connection_cancel(connection)
    return
  }
  let state = ServiceConnectionState(connection: connection)
  retainServiceConnection(state)
  xpc_connection_set_event_handler(connection) { [weak state] message in
    guard let state else { return }
    handleServiceMessage(state: state, message: message)
  }
  xpc_connection_activate(connection)
}

private func handleServiceMessage(state: ServiceConnectionState, message: xpc_object_t) {
  guard xpc_get_type(message) == XPC_TYPE_DICTIONARY else {
    if xpc_get_type(message) == XPC_TYPE_ERROR {
      handlePersistentServiceConnectionFailure(state)
    }
    return
  }
  switch dictionaryString(message, key: "kind") {
  case "sessionHello":
    serviceSessionHello(state: state, message: message)
  case "sessionBegin":
    serviceSessionBegin(state: state, message: message)
  case "sessionEnd":
    serviceSessionEnd(state: state, message: message)
  case "launch":
    serviceLaunch(state: state, message: message)
  case "resume":
    serviceResume(state: state, message: message)
  case "cancel":
    serviceCancel(state: state, message: message)
  default:
    sendServiceError(message, diagnostic: "unknown typed Git XPC message")
  }
}

private func serviceLaunch(state: ServiceConnectionState, message: xpc_object_t) {
  let requestID = xpc_dictionary_get_uint64(message, "requestId")
  let nonceHigh = xpc_dictionary_get_uint64(message, "nonceHigh")
  let nonceLow = xpc_dictionary_get_uint64(message, "nonceLow")
  let familyValue = xpc_dictionary_get_uint64(message, "family")
  guard requestID != 0, familyValue > 0, familyValue <= UInt64(UInt8.max),
    let payloadPointer = xpc_dictionary_get_string(message, "payload")
  else {
    sendServiceError(message, diagnostic: "invalid typed Git XPC launch envelope")
    return
  }
  guard state.matchesSession(message), state.reserveLaunch(
    sessionID: xpc_dictionary_get_uint64(message, "sessionId"),
    sessionNonceHigh: xpc_dictionary_get_uint64(message, "sessionNonceHigh"),
    sessionNonceLow: xpc_dictionary_get_uint64(message, "sessionNonceLow"),
    requestID: requestID,
    nonceHigh: nonceHigh,
    nonceLow: nonceLow
  ) else {
    sendLaunchRejected(
      state: state,
      request: message,
      requestID: requestID,
      nonceHigh: nonceHigh,
      nonceLow: nonceLow,
      diagnostic: "typed Git XPC launch phase did not match"
    )
    return
  }
  guard state.validateSessionGitReservation() else {
    state.quarantinePersistentSession()
    sendServiceQuarantined(
      state: state,
      request: message,
      kind: "launchQuarantined",
      requestID: requestID,
      nonceHigh: nonceHigh,
      nonceLow: nonceLow,
      diagnostic: "fixed system Git reservation changed before launch"
    )
    return
  }
  let reject: (String) -> Void = { diagnostic in
    guard sendLaunchRejected(
      state: state,
      request: message,
      requestID: requestID,
      nonceHigh: nonceHigh,
      nonceLow: nonceLow,
      diagnostic: diagnostic,
      resetChild: true
    ) else {
      state.quarantineServiceAuthority()
      sendServiceQuarantined(
        state: state, request: message, kind: "launchQuarantined",
        requestID: requestID, nonceHigh: nonceHigh, nonceLow: nonceLow,
        diagnostic: "typed Git rejected child could not reset cleanup proof"
      )
      return
    }
  }
  let payload = String(cString: payloadPointer)
  guard payload.utf8.count <= maximumPayloadBytes else {
    reject("typed Git XPC payload exceeded its limit")
    return
  }

  let cwdFD = xpc_dictionary_dup_fd(message, "cwdFd")
  let stdinFD = xpc_dictionary_dup_fd(message, "stdinFd")
  let stdoutFD = xpc_dictionary_dup_fd(message, "stdoutFd")
  let stderrFD = xpc_dictionary_dup_fd(message, "stderrFd")
  let slotFD = xpc_dictionary_dup_fd(message, "pidSlotFd")
  let descriptors = [cwdFD, stdinFD, stdoutFD, stderrFD, slotFD]
  guard descriptors.allSatisfy({ $0 >= 0 }) else {
    descriptors.filter({ $0 >= 0 }).forEach({ _ = close($0) })
    reject("typed Git XPC launch omitted a descriptor")
    return
  }
  defer { descriptors.forEach({ _ = close($0) }) }

  guard let cwd = descriptorStat(cwdFD), let stdin = descriptorStat(stdinFD),
    (cwd.mode & UInt32(S_IFMT)) == UInt32(S_IFDIR)
  else {
    reject("typed Git XPC cwd was not an opened directory")
    return
  }
  let prepared = schoolx_git_xpc_prepare(
    UInt8(familyValue),
    RustString(payload),
    cwd.device,
    cwd.inode,
    cwd.mode,
    stdin.device,
    stdin.inode,
    stdin.mode,
    stdin.size
  ).toString()
  guard prepared.utf8.count <= maximumPayloadBytes,
    let data = prepared.data(using: .utf8),
    let response = try? JSONDecoder().decode(PreparedResponse.self, from: data),
    response.ok,
    let spec = response.spec
  else {
    let diagnostic = decodePrepareError(prepared)
    reject(diagnostic)
    return
  }

  let slotLength = Int(sysconf(_SC_PAGESIZE))
  guard slotLength >= MemoryLayout<Int32>.size,
    validatePIDSlot(
      fd: slotFD,
      length: slotLength,
      nonceHigh: nonceHigh,
      nonceLow: nonceLow
    ),
    let slot = mapPIDSlot(fd: slotFD, length: slotLength)
  else {
    reject("failed to map typed Git PID authority")
    return
  }
  let retainedSlotFD = fcntl(slotFD, F_DUPFD_CLOEXEC, 3)
  guard retainedSlotFD >= 0 else {
    _ = munmap(UnsafeMutableRawPointer(slot), slotLength)
    reject("failed to retain typed Git PID authority")
    return
  }
  guard state.installPIDSlot(fd: retainedSlotFD, slot: slot, length: slotLength) else {
    _ = munmap(UnsafeMutableRawPointer(slot), slotLength)
    _ = close(retainedSlotFD)
    reject("failed to install typed Git cleanup authority")
    return
  }

  let holdLaunchAuthority: (String) -> Void = { diagnostic in
    sendServiceQuarantined(
      state: state,
      request: message,
      kind: "launchQuarantined",
      requestID: requestID,
      nonceHigh: nonceHigh,
      nonceLow: nonceLow,
      diagnostic: diagnostic
    )
    if state.currentOwnerState() == .clientAborting {
      _ = state.armClientCleanupObservation()
    }
  }
  let quarantineLaunch: (String) -> Void = { diagnostic in
    state.quarantineServiceAuthority()
    holdLaunchAuthority(diagnostic)
  }
  switch state.claimLaunchOwner() {
  case .acquired:
    break
  case .cleanupProven:
    guard state.acceptCleanupProof(error: nil) else {
      quarantineLaunch("typed Git no-child cleanup proof could not be adopted")
      return
    }
    reject("typed Git launch was cancelled before process creation")
    return
  case .clientAborting:
    holdLaunchAuthority("typed Git client cleanup won before process creation")
    return
  case .serviceCleaning:
    holdLaunchAuthority("typed Git orphan cleanup began before process creation")
    return
  case .invalid:
    quarantineLaunch("typed Git launch cleanup ownership was invalid")
    return
  }
  guard state.sealLaunchOwnerForSpawn() else {
    switch state.currentOwnerState() {
    case .clientAborting:
      holdLaunchAuthority("typed Git client cleanup won before process creation")
    case .cleanupProven:
      if state.acceptCleanupProof(error: nil) {
        reject("typed Git launch was cancelled before process creation")
      } else {
        quarantineLaunch("typed Git no-child cleanup proof could not be adopted")
      }
    default:
      quarantineLaunch("typed Git spawn ownership could not be sealed")
    }
    return
  }

  let spawnResult = spawnSuspendedGit(
    spec: spec,
    cwdFD: cwdFD,
    stdinFD: stdinFD,
    stdoutFD: stdoutFD,
    stderrFD: stderrFD,
    pidSlot: slot
  )
  guard case .success(let pid) = spawnResult else {
    let diagnostic: String
    if case .failure(let error) = spawnResult {
      diagnostic = error
    } else {
      diagnostic = "typed Git posix_spawn failed without a result"
    }
    switch state.beginSpawnFailureCleanup(pid: 0) {
    case .kill:
      if state.proveCleanup(error: nil) {
        reject(diagnostic)
      } else {
        quarantineLaunch(diagnostic)
      }
    case .cleanupProven:
      if state.acceptCleanupProof(error: nil) {
        reject(diagnostic)
      } else {
        quarantineLaunch(diagnostic)
      }
    case .awaitCleanup, .clientTakeover, .noAuthority, .invalid:
      quarantineLaunch(diagnostic)
    }
    return
  }
  let helperSession = getsid(0)
  guard state.installSpawnedPID(pid) else {
    quarantineLaunch("typed Git spawned PID authority could not be installed")
    return
  }
  let processIdentityMatches = slot.pointee == pid && getpgid(pid) == pid
    && helperSession > 0 && getsid(pid) == helperSession
  guard processIdentityMatches else {
    let diagnostic = "typed Git process group identity did not match"
    if killAndProveServiceSpawnFailure(state: state, pid: pid) {
      reject(diagnostic)
    } else {
      startServiceReaper(
        state: state,
        requestID: requestID,
        nonceHigh: nonceHigh,
        nonceLow: nonceLow,
        pid: pid
      )
      quarantineLaunch(diagnostic)
    }
    return
  }

  switch state.currentOwnerState() {
  case .serviceCleaning:
    break
  case .clientAborting:
    startServiceReaper(
      state: state,
      requestID: requestID,
      nonceHigh: nonceHigh,
      nonceLow: nonceLow,
      pid: pid
    )
    quarantineLaunch("typed Git client cleanup won during process creation")
    return
  default:
    startServiceReaper(
      state: state,
      requestID: requestID,
      nonceHigh: nonceHigh,
      nonceLow: nonceLow,
      pid: pid
    )
    quarantineLaunch("typed Git launch owner changed before Started")
    return
  }
  startServiceReaper(
    state: state,
    requestID: requestID,
    nonceHigh: nonceHigh,
    nonceLow: nonceLow,
    pid: pid
  )
  DispatchQueue.global(qos: .userInitiated).asyncAfter(deadline: .now() + .seconds(5)) {
    let disposition = state.resumeDeadlineDisposition(
      requestID: requestID,
      nonceHigh: nonceHigh,
      nonceLow: nonceLow
    )
    if case .kill(let timedOutPID) = disposition { killProcessGroup(timedOutPID) }
  }
  guard let reply = xpc_dictionary_create_reply(message) else {
    if case .kill(let pid) = state.beginCleanup(pid: pid) { killProcessGroup(pid) }
    return
  }
  xpc_dictionary_set_string(reply, "kind", "started")
  setServiceSessionEnvelope(reply, state: state)
  xpc_dictionary_set_uint64(reply, "requestId", requestID)
  xpc_dictionary_set_uint64(reply, "nonceHigh", nonceHigh)
  xpc_dictionary_set_uint64(reply, "nonceLow", nonceLow)
  xpc_dictionary_set_int64(reply, "pid", Int64(pid))
  xpc_dictionary_set_int64(reply, "helperPid", Int64(getpid()))
  xpc_dictionary_set_int64(reply, "helperSid", Int64(helperSession))
  xpc_connection_send_message(state.connection, reply)
}

private func serviceResume(state: ServiceConnectionState, message: xpc_object_t) {
  let requestID = xpc_dictionary_get_uint64(message, "requestId")
  let nonceHigh = xpc_dictionary_get_uint64(message, "nonceHigh")
  let nonceLow = xpc_dictionary_get_uint64(message, "nonceLow")
  guard state.matchesSession(message), let pid = state.markResumed(
    requestID: requestID,
    nonceHigh: nonceHigh,
    nonceLow: nonceLow
  ) else {
    sendServiceError(message, diagnostic: "typed Git resume authority did not match")
    return
  }
  guard kill(pid, SIGCONT) == 0 else {
    if case .kill(let cleanupPID) = state.beginCleanup(pid: pid) {
      killProcessGroup(cleanupPID)
    }
    sendServiceError(message, diagnostic: "typed Git resume failed")
    return
  }
  guard let reply = xpc_dictionary_create_reply(message) else {
    if case .kill(let cleanupPID) = state.beginCleanup(pid: pid) {
      killProcessGroup(cleanupPID)
    }
    return
  }
  xpc_dictionary_set_string(reply, "kind", "resumed")
  setServiceSessionEnvelope(reply, state: state)
  xpc_dictionary_set_uint64(reply, "requestId", requestID)
  xpc_dictionary_set_uint64(reply, "nonceHigh", nonceHigh)
  xpc_dictionary_set_uint64(reply, "nonceLow", nonceLow)
  xpc_connection_send_message(state.connection, reply)
}

private func serviceCancel(state: ServiceConnectionState, message: xpc_object_t) {
  let requestID = xpc_dictionary_get_uint64(message, "requestId")
  let nonceHigh = xpc_dictionary_get_uint64(message, "nonceHigh")
  let nonceLow = xpc_dictionary_get_uint64(message, "nonceLow")
  guard state.matchesSession(message), let disposition = state.beginCancel(
    requestID: requestID,
    nonceHigh: nonceHigh,
    nonceLow: nonceLow,
    request: message
  ) else {
    sendServiceError(message, diagnostic: "typed Git cancel authority did not match")
    return
  }
  switch disposition {
  case .kill(let pid):
    killProcessGroup(pid)
  case .awaitCleanup, .clientTakeover:
    break
  case .completed(let error):
    sendCancelAcknowledgment(
      state: state,
      request: message,
      requestID: requestID,
      nonceHigh: nonceHigh,
      nonceLow: nonceLow,
      error: error
    )
  }
}

func reapGitProcessGroup(
  state: ServiceConnectionState,
  requestID: UInt64,
  nonceHigh: UInt64,
  nonceLow: UInt64,
  pid: Int32
) {
  var info = siginfo_t()
  var diagnostic: String?
  while true {
    if waitid(P_PID, id_t(pid), &info, WEXITED | WNOWAIT) == 0 { break }
    if errno == EINTR { continue }
    diagnostic = "waitid failed for typed Git: \(errnoDiagnostic())"
    break
  }
  let disposition = state.beginCleanup(pid: pid)
  switch disposition {
  case .kill(let cleanupPID):
    killProcessGroup(cleanupPID)
  case .awaitCleanup:
    break
  case .clientTakeover:
    if state.yieldServiceReaperToClient() { return }
  case .cleanupProven:
    return
  case .noAuthority, .invalid:
    quarantineServiceCleanup(
      state: state,
      requestID: requestID,
      nonceHigh: nonceHigh,
      nonceLow: nonceLow,
      diagnostic: diagnostic ?? "typed Git cleanup ownership was invalid"
    )
    return
  }
  guard state.markPIDSlotTerminal() else {
    quarantineServiceCleanup(
      state: state,
      requestID: requestID,
      nonceHigh: nonceHigh,
      nonceLow: nonceLow,
      diagnostic: "typed Git service could not mark its PID authority terminal"
    )
    return
  }
  var rawStatus: Int32 = 0
  var reaped = false
  if diagnostic == nil {
    while true {
      let result = waitpid(pid, &rawStatus, 0)
      if result == pid {
        reaped = true
        break
      }
      if result < 0, errno == EINTR { continue }
      diagnostic = appendDiagnostic(
        diagnostic,
        "waitpid failed for typed Git: \(errnoDiagnostic())"
      )
      break
    }
  }
  guard waitForProcessGroupExit(pid) else {
    quarantineServiceCleanup(
      state: state,
      requestID: requestID,
      nonceHigh: nonceHigh,
      nonceLow: nonceLow,
      diagnostic: appendDiagnostic(
        diagnostic,
        "typed Git process group remained alive after cleanup"
      )
    )
    return
  }
  guard state.proveCleanup(error: diagnostic) else {
    quarantineServiceCleanup(
      state: state,
      requestID: requestID,
      nonceHigh: nonceHigh,
      nonceLow: nonceLow,
      diagnostic: "typed Git cleanup proof ownership was lost"
    )
    return
  }
  sendFinished(
    state: state,
    requestID: requestID,
    nonceHigh: nonceHigh,
    nonceLow: nonceLow,
    rawStatus: reaped ? rawStatus : 0,
    error: diagnostic
  )
}
