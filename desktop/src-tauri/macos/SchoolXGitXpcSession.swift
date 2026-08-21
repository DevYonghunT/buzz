import Darwin
import Dispatch
import Foundation
import XPC

let rootTrustedGitPath = "/usr/bin/git"

struct EncodedSessionResult: Encodable {
  let ok: Bool
  let sessionId: UInt64
  let sessionCleanupProven: Bool
  let sessionAuthorityRetained: Bool
  var error = ""
}

struct EncodedChildResult: Encodable {
  let ok: Bool
  var pid: UInt32 = 0
  let childCleanupProven: Bool
  let childAuthorityRetained: Bool
  var error = ""
}

struct EncodedPollFailure: Encodable {
  let state = "failed"
  let error: String
  let childCleanupProven: Bool
  let childAuthorityRetained: Bool
}

private enum GitReservationResult {
  case success(Int32)
  case failure(String)
}

fileprivate enum ClientSessionEndDisposition {
  case send
  case recoverLocally
  case retained(String)
}

private enum SessionAdmissionDisposition {
  case admitted
  case rejected(String)
  case invalid
}

final class ClientAuthoritySession {
  let sessionID: UInt64
  let nonceHigh: UInt64
  let nonceLow: UInt64
  let connection: xpc_connection_t
  let queue: DispatchQueue
  let lock = NSLock()
  var gitFD: Int32
  var helperPID: Int32 = 0
  var helperPGID: Int32 = 0
  var helperSID: Int32 = 0
  var helperIncarnationHigh: UInt64 = 0
  var helperIncarnationLow: UInt64 = 0
  var helperExitProven = false
  var helloInstalled = false
  var beginSent = false
  var admitted = false
  var ending = false
  var lateCleanupArmed = false
  var quarantined = false
  var cleanupProven = false
  var activeRequestID: UInt64 = 0
  var retiredRequestID: UInt64 = 0
  var retiredNonceHigh: UInt64 = 0
  var retiredNonceLow: UInt64 = 0
  var helperExitSource: DispatchSourceProcess?

  init(
    sessionID: UInt64,
    nonceHigh: UInt64,
    nonceLow: UInt64,
    connection: xpc_connection_t,
    queue: DispatchQueue,
    gitFD: Int32
  ) {
    self.sessionID = sessionID
    self.nonceHigh = nonceHigh
    self.nonceLow = nonceLow
    self.connection = connection
    self.queue = queue
    self.gitFD = gitFD
  }

  deinit {
    helperExitSource?.cancel()
    // An unexpected deinit is not an authority-release event. The descriptor
    // intentionally remains open until process exit unless clean end proves it.
  }

  func installHelper(
    pid: Int32,
    pgid: Int32,
    sid: Int32,
    incarnationHigh: UInt64,
    incarnationLow: UInt64
  ) -> Bool {
    guard pid > 1, pgid == pid, sid > 0,
      incarnationHigh != 0, incarnationLow != 0,
      xpc_connection_get_pid(connection) == pid,
      getpgid(pid) == pgid, getsid(pid) == sid
    else { return false }
    let source = DispatchSource.makeProcessSource(
      identifier: pid,
      eventMask: .exit,
      queue: queue
    )
    source.setEventHandler { [weak self] in self?.authenticatedHelperDidExit() }
    lock.lock()
    guard !helloInstalled, helperExitSource == nil else {
      lock.unlock()
      source.resume()
      source.cancel()
      return false
    }
    helperPID = pid
    helperPGID = pgid
    helperSID = sid
    helperIncarnationHigh = incarnationHigh
    helperIncarnationLow = incarnationLow
    helloInstalled = true
    helperExitSource = source
    lock.unlock()
    source.resume()
    return true
  }

  func markBeginSent() -> Bool {
    lock.lock()
    defer { lock.unlock() }
    guard helloInstalled, !beginSent, !quarantined else { return false }
    beginSent = true
    return true
  }

  func markAdmitted() -> Bool {
    lock.lock()
    defer { lock.unlock() }
    guard beginSent, !admitted, !ending, !quarantined else { return false }
    admitted = true
    return true
  }

  func reserveChild(_ requestID: UInt64) -> Bool {
    lock.lock()
    defer { lock.unlock() }
    guard admitted, !ending, !quarantined, activeRequestID == 0, requestID != 0
    else { return false }
    activeRequestID = requestID
    return true
  }

  func releaseChild(_ operation: ClientOperation) {
    lock.lock()
    if activeRequestID == operation.requestID {
      activeRequestID = 0
      retiredRequestID = operation.requestID
      retiredNonceHigh = operation.nonceHigh
      retiredNonceLow = operation.nonceLow
    }
    lock.unlock()
  }

  func matchesRetiredFinished(_ message: xpc_object_t) -> Bool {
    lock.lock()
    defer { lock.unlock() }
    return dictionaryString(message, key: "kind") == "finished"
      && retiredRequestID == xpc_dictionary_get_uint64(message, "requestId")
      && retiredNonceHigh == xpc_dictionary_get_uint64(message, "nonceHigh")
      && retiredNonceLow == xpc_dictionary_get_uint64(message, "nonceLow")
  }

  func childAuthorityRetained(_ requestID: UInt64) {
    lock.lock()
    if activeRequestID == requestID { quarantined = true }
    lock.unlock()
  }

  fileprivate func beginEnd() -> ClientSessionEndDisposition {
    lock.lock()
    defer { lock.unlock() }
    if admitted, helperExitProven, activeRequestID == 0, !ending {
      ending = true
      return .recoverLocally
    }
    guard admitted, !ending, !quarantined else {
      return .retained("signed XPC session is not safely endable")
    }
    guard activeRequestID == 0 else {
      quarantined = true
      return .retained("signed XPC session still has child authority")
    }
    ending = true
    return .send
  }

  func quarantine(_ diagnostic: String) {
    lock.lock()
    quarantined = true
    let requestID = activeRequestID
    lock.unlock()
    if requestID != 0 {
      lookupClientOperation(requestID)?.fail(diagnostic)
    }
  }

  func connectionFailed(_ diagnostic: String) {
    lock.lock()
    let requestID = activeRequestID
    let ambiguous = beginSent || admitted
    if ambiguous { quarantined = true }
    lock.unlock()
    if requestID != 0 {
      lookupClientOperation(requestID)?.fail(diagnostic)
    }
  }

  func authenticatedHelperDidExit() {
    lock.lock()
    helperExitProven = true
    quarantined = true
    let requestID = activeRequestID
    let lateFD = lateCleanupArmed && requestID == 0
      ? claimHelperExitCleanupLocked() : nil
    lock.unlock()
    if requestID != 0 {
      lookupClientOperation(requestID)?.authenticatedSessionHelperDidExit()
    } else if let lateFD, completeHelperExitCleanup(lateFD) {
      removeClientSession(self)
      _ = schoolx_git_xpc_session_cleanup_proven(sessionID)
    }
  }

  func helperIdentity() -> (Int32, Int32, Int32)? {
    lock.lock()
    defer { lock.unlock() }
    guard helloInstalled else { return nil }
    return (helperPID, helperPGID, helperSID)
  }

  func matchesEnvelope(
    sessionID: UInt64,
    nonceHigh: UInt64,
    nonceLow: UInt64,
    incarnationHigh: UInt64,
    incarnationLow: UInt64
  ) -> Bool {
    lock.lock()
    defer { lock.unlock() }
    return self.sessionID == sessionID && self.nonceHigh == nonceHigh
      && self.nonceLow == nonceLow && helperIncarnationHigh == incarnationHigh
      && helperIncarnationLow == incarnationLow
  }

  func incarnation() -> (UInt64, UInt64) {
    lock.lock()
    defer { lock.unlock() }
    return (helperIncarnationHigh, helperIncarnationLow)
  }

  func releaseAfterCleanEnd() -> Bool {
    lock.lock()
    guard ending, activeRequestID == 0, gitFD >= 0 else {
      quarantined = true
      lock.unlock()
      return false
    }
    let fd = gitFD
    gitFD = -1
    admitted = false
    lock.unlock()
    guard flock(fd, LOCK_UN) == 0 else {
      lock.lock()
      quarantined = true
      lock.unlock()
      return false
    }
    lock.lock()
    cleanupProven = true
    quarantined = false
    lock.unlock()
    _ = close(fd)
    helperExitSource?.cancel()
    xpc_connection_cancel(connection)
    return true
  }

  func releaseAfterHelperExitProof() -> Bool {
    lock.lock()
    let fd = claimHelperExitCleanupLocked()
    lock.unlock()
    return fd.map(completeHelperExitCleanup) ?? false
  }

  private func claimHelperExitCleanupLocked() -> Int32? {
    let endRecovery = admitted && ending
    let admissionRecovery = beginSent && !admitted
    guard helperExitProven, activeRequestID == 0, gitFD >= 0,
      endRecovery || admissionRecovery
    else { return nil }
    let fd = gitFD
    gitFD = -1
    return fd
  }

  private func completeHelperExitCleanup(_ fd: Int32) -> Bool {
    guard flock(fd, LOCK_UN) == 0 else { return false }
    _ = close(fd)
    lock.lock()
    admitted = false
    lateCleanupArmed = false
    cleanupProven = true
    quarantined = false
    lock.unlock()
    helperExitSource?.cancel()
    xpc_connection_cancel(connection)
    return true
  }

  func armLateHelperExitCleanup() -> Bool {
    lock.lock()
    lateCleanupArmed = true
    quarantined = true
    let fd = claimHelperExitCleanupLocked()
    lock.unlock()
    return fd.map(completeHelperExitCleanup) ?? false
  }
}

private let clientSessionsLock = NSLock()
private var clientSessions: [UInt64: ClientAuthoritySession] = [:]

func lookupClientSession(_ sessionID: UInt64) -> ClientAuthoritySession? {
  clientSessionsLock.lock()
  defer { clientSessionsLock.unlock() }
  return clientSessions[sessionID]
}

private func installClientSession(_ session: ClientAuthoritySession) -> Bool {
  clientSessionsLock.lock()
  defer { clientSessionsLock.unlock() }
  guard clientSessions.isEmpty, clientSessions[session.sessionID] == nil else { return false }
  clientSessions[session.sessionID] = session
  return true
}

private func removeClientSession(_ session: ClientAuthoritySession) {
  clientSessionsLock.lock()
  if clientSessions[session.sessionID] === session {
    clientSessions.removeValue(forKey: session.sessionID)
  }
  clientSessionsLock.unlock()
}

func schoolx_git_xpc_session_begin(session_id: UInt64) -> RustString {
  guard session_id != 0 else {
    return encodeSessionFailure(sessionID: session_id, "invalid XPC session identifier")
  }
  if let diagnostic = capabilityDiagnostic() {
    return encodeSessionFailure(sessionID: session_id, diagnostic)
  }
  let reservation = openGitReservation()
  guard case .success(let gitFD) = reservation else {
    if case .failure(let diagnostic) = reservation {
      return encodeSessionFailure(sessionID: session_id, diagnostic)
    }
    return encodeSessionFailure(sessionID: session_id, "failed to reserve system Git")
  }
  let nonceHigh = randomNonceComponent()
  let nonceLow = randomNonceComponent()
  let queue = DispatchQueue(label: "schoolx.code.git.xpc.session.\(session_id)")
  let connection = xpc_connection_create(schoolXGitServiceIdentifier, queue)
  let session = ClientAuthoritySession(
    sessionID: session_id,
    nonceHigh: nonceHigh,
    nonceLow: nonceLow,
    connection: connection,
    queue: queue,
    gitFD: gitFD
  )
  guard installClientSession(session) else {
    _ = flock(gitFD, LOCK_UN)
    _ = close(gitFD)
    return encodeSessionFailure(sessionID: session_id, "another XPC session is active")
  }
  guard #available(macOS 12.0, *),
    let identity = currentSigningIdentity(),
    installPeerRequirement(
      on: connection,
      expectedIdentifier: schoolXGitServiceIdentifier,
      teamIdentifier: identity.teamIdentifier
    ) == nil
  else {
    _ = session.releaseAfterPreAdmissionFailure()
    removeClientSession(session)
    return encodeSessionFailure(sessionID: session_id, "failed to authenticate XPC helper")
  }
  xpc_connection_set_event_handler(connection) { [weak session] event in
    guard let session else { return }
    handleClientSessionEvent(session: session, event: event)
  }
  xpc_connection_activate(connection)

  let hello = xpc_dictionary_create(nil, nil, 0)
  xpc_dictionary_set_string(hello, "kind", "sessionHello")
  setSessionEnvelope(hello, session: session)
  xpc_dictionary_set_uint64(hello, "protocolVersion", protocolVersion)
  let helloReply = ReplyBox()
  xpc_connection_send_message_with_reply(connection, hello, queue) { helloReply.store($0) }
  guard helloReply.wait(timeout: helloTimeout), let reply = helloReply.take(),
    parseSessionHelloReply(reply, session: session), session.markBeginSent()
  else {
    _ = session.releaseAfterPreAdmissionFailure()
    removeClientSession(session)
    return encodeSessionFailure(sessionID: session_id, "signed XPC session handshake failed")
  }

  let begin = xpc_dictionary_create(nil, nil, 0)
  xpc_dictionary_set_string(begin, "kind", "sessionBegin")
  setSessionEnvelope(begin, session: session)
  xpc_dictionary_set_fd(begin, "gitFd", gitFD)
  let beginReply = ReplyBox()
  xpc_connection_send_message_with_reply(connection, begin, queue) { beginReply.store($0) }
  guard beginReply.wait(timeout: launchTimeout), let response = beginReply.take() else {
    if session.armLateHelperExitCleanup() {
      removeClientSession(session)
      return encodeSessionFailure(sessionID: session_id, "signed XPC helper exited during admission")
    }
    return encodeSessionRetained(sessionID: session_id, "signed XPC session admission timed out")
  }
  switch parseSessionAdmissionReply(response, session: session) {
  case .admitted:
    break
  case .rejected(let diagnostic):
    guard session.releaseAfterCleanAdmissionRejection() else {
      session.quarantine("signed XPC session rejection could not release local authority")
      return encodeSessionRetained(
        sessionID: session_id,
        "signed XPC session rejection could not release local authority"
      )
    }
    removeClientSession(session)
    return encodeSessionFailure(sessionID: session_id, diagnostic)
  case .invalid:
    if session.armLateHelperExitCleanup() {
      removeClientSession(session)
      return encodeSessionFailure(
        sessionID: session_id,
        "signed XPC helper exited with no child during admission"
      )
    }
    return encodeSessionRetained(
      sessionID: session_id,
      "signed XPC session admission response was invalid"
    )
  }
  return encodeRustString(
    EncodedSessionResult(
      ok: true,
      sessionId: session_id,
      sessionCleanupProven: false,
      sessionAuthorityRetained: true
    ))
}

func schoolx_git_xpc_session_end(session_id: UInt64) -> RustString {
  guard let session = lookupClientSession(session_id) else {
    return encodeSessionRetained(sessionID: session_id, "unknown XPC session")
  }
  switch session.beginEnd() {
  case .recoverLocally:
    guard session.releaseAfterHelperExitProof() else {
      return encodeSessionRetained(sessionID: session_id, "helper-exit recovery was ambiguous")
    }
    removeClientSession(session)
    return encodeSessionFailure(sessionID: session_id, "signed XPC helper exited before session end")
  case .retained(let diagnostic):
    return encodeSessionRetained(sessionID: session_id, diagnostic)
  case .send:
    break
  }
  let end = xpc_dictionary_create(nil, nil, 0)
  xpc_dictionary_set_string(end, "kind", "sessionEnd")
  setSessionEnvelope(end, session: session)
  let replyBox = ReplyBox()
  xpc_connection_send_message_with_reply(session.connection, end, session.queue) {
    replyBox.store($0)
  }
  guard replyBox.wait(timeout: cancelTimeout), let reply = replyBox.take(),
    parseSessionEndReply(reply, session: session), session.releaseAfterCleanEnd()
  else {
    if session.armLateHelperExitCleanup() {
      removeClientSession(session)
      return encodeSessionFailure(
        sessionID: session_id,
        "signed XPC helper exited during session end"
      )
    }
    return encodeSessionRetained(
      sessionID: session_id,
      "signed XPC session end lacked exact cleanup proof"
    )
  }
  removeClientSession(session)
  return encodeRustString(
    EncodedSessionResult(
      ok: true,
      sessionId: session_id,
      sessionCleanupProven: true,
      sessionAuthorityRetained: false
    ))
}

func setSessionEnvelope(_ message: xpc_object_t, session: ClientAuthoritySession) {
  let incarnation = session.incarnation()
  xpc_dictionary_set_uint64(message, "sessionId", session.sessionID)
  xpc_dictionary_set_uint64(message, "sessionNonceHigh", session.nonceHigh)
  xpc_dictionary_set_uint64(message, "sessionNonceLow", session.nonceLow)
  xpc_dictionary_set_uint64(message, "helperIncarnationHigh", incarnation.0)
  xpc_dictionary_set_uint64(message, "helperIncarnationLow", incarnation.1)
}

func setSessionEnvelope(_ message: xpc_object_t, operation: ClientOperation) {
  setSessionEnvelope(message, session: operation.session)
}

func messageMatchesSession(_ message: xpc_object_t, operation: ClientOperation) -> Bool {
  messageMatchesSession(message, session: operation.session)
}

func encodeChildFailure(
  _ error: String,
  _ operation: ClientOperation? = nil,
  cleanupProvenWithoutOperation: Bool = true
) -> RustString {
  let disposition = operation?.childAuthorityDisposition()
    ?? (
      proven: cleanupProvenWithoutOperation,
      retained: !cleanupProvenWithoutOperation
    )
  if disposition.retained, let operation {
    operation.session.childAuthorityRetained(operation.requestID)
  }
  return encodeRustString(
    EncodedChildResult(
      ok: false,
      childCleanupProven: disposition.proven,
      childAuthorityRetained: disposition.retained,
      error: error
    ))
}

func handleClientSessionEvent(
  session: ClientAuthoritySession,
  event: xpc_object_t
) {
  if xpc_get_type(event) == XPC_TYPE_ERROR {
    session.connectionFailed(
      "signed XPC helper connection failed: \(xpcDescription(event))"
    )
    return
  }
  guard xpc_get_type(event) == XPC_TYPE_DICTIONARY,
    messageMatchesSession(event, session: session)
  else {
    session.quarantine("signed XPC helper sent an invalid session event")
    return
  }
  let requestID = xpc_dictionary_get_uint64(event, "requestId")
  guard requestID != 0, let operation = lookupClientOperation(requestID),
    operation.session === session
  else {
    if session.matchesRetiredFinished(event) { return }
    session.quarantine("signed XPC helper changed child session authority")
    return
  }
  handleClientEvent(operation: operation, event: event)
}

private func parseSessionHelloReply(
  _ reply: xpc_object_t,
  session: ClientAuthoritySession
) -> Bool {
  guard xpc_get_type(reply) == XPC_TYPE_DICTIONARY,
    dictionaryString(reply, key: "kind") == "sessionHelloAck",
    xpc_dictionary_get_uint64(reply, "protocolVersion") == protocolVersion,
    messageMatchesSessionToken(reply, session: session)
  else { return false }
  let pid = xpc_dictionary_get_int64(reply, "helperPid")
  let pgid = xpc_dictionary_get_int64(reply, "helperPgid")
  let sid = xpc_dictionary_get_int64(reply, "helperSid")
  guard pid > 1, pid <= Int64(Int32.max), pgid == pid,
    sid > 0, sid <= Int64(Int32.max)
  else { return false }
  return session.installHelper(
    pid: Int32(pid),
    pgid: Int32(pgid),
    sid: Int32(sid),
    incarnationHigh: xpc_dictionary_get_uint64(reply, "helperIncarnationHigh"),
    incarnationLow: xpc_dictionary_get_uint64(reply, "helperIncarnationLow")
  )
}

private func parseSessionAdmissionReply(
  _ reply: xpc_object_t,
  session: ClientAuthoritySession
) -> SessionAdmissionDisposition {
  guard xpc_get_type(reply) == XPC_TYPE_DICTIONARY,
    dictionaryString(reply, key: "kind") == "sessionBegan",
    messageMatchesSession(reply, session: session)
  else { return .invalid }
  let ok = xpc_dictionary_get_bool(reply, "ok")
  let proven = xpc_dictionary_get_bool(reply, "sessionCleanupProven")
  let retained = xpc_dictionary_get_bool(reply, "sessionAuthorityRetained")
  if ok, !proven, retained, session.markAdmitted() { return .admitted }
  if !ok, proven, !retained {
    return .rejected(
      dictionaryString(reply, key: "error") ?? "signed XPC session admission was rejected"
    )
  }
  return .invalid
}

private func parseSessionEndReply(
  _ reply: xpc_object_t,
  session: ClientAuthoritySession
) -> Bool {
  xpc_get_type(reply) == XPC_TYPE_DICTIONARY
    && dictionaryString(reply, key: "kind") == "sessionEnded"
    && messageMatchesSession(reply, session: session)
    && xpc_dictionary_get_bool(reply, "sessionCleanupProven")
    && !xpc_dictionary_get_bool(reply, "sessionAuthorityRetained")
}

private func messageMatchesSession(
  _ message: xpc_object_t,
  session: ClientAuthoritySession
) -> Bool {
  session.matchesEnvelope(
    sessionID: xpc_dictionary_get_uint64(message, "sessionId"),
    nonceHigh: xpc_dictionary_get_uint64(message, "sessionNonceHigh"),
    nonceLow: xpc_dictionary_get_uint64(message, "sessionNonceLow"),
    incarnationHigh: xpc_dictionary_get_uint64(message, "helperIncarnationHigh"),
    incarnationLow: xpc_dictionary_get_uint64(message, "helperIncarnationLow")
  )
}

private func messageMatchesSessionToken(
  _ message: xpc_object_t,
  session: ClientAuthoritySession
) -> Bool {
  xpc_dictionary_get_uint64(message, "sessionId") == session.sessionID
    && xpc_dictionary_get_uint64(message, "sessionNonceHigh") == session.nonceHigh
    && xpc_dictionary_get_uint64(message, "sessionNonceLow") == session.nonceLow
}

private func encodeSessionFailure(sessionID: UInt64, _ error: String) -> RustString {
  encodeRustString(
    EncodedSessionResult(
      ok: false,
      sessionId: sessionID,
      sessionCleanupProven: true,
      sessionAuthorityRetained: false,
      error: error
    ))
}

private func encodeSessionRetained(sessionID: UInt64, _ error: String) -> RustString {
  encodeRustString(
    EncodedSessionResult(
      ok: false,
      sessionId: sessionID,
      sessionCleanupProven: false,
      sessionAuthorityRetained: true,
      error: error
    ))
}

private func openGitReservation() -> GitReservationResult {
  let fd = open(rootTrustedGitPath, O_RDONLY | O_NOFOLLOW | O_CLOEXEC)
  guard fd >= 0 else {
    return .failure("failed to open fixed system Git: \(errnoDiagnostic())")
  }
  guard validateGitReservationFD(fd), flock(fd, LOCK_EX | LOCK_NB) == 0,
    validateGitReservationFD(fd)
  else {
    let diagnostic = errnoDiagnostic()
    _ = close(fd)
    return .failure("failed to reserve fixed system Git: \(diagnostic)")
  }
  return .success(fd)
}

private extension ClientAuthoritySession {
  func releaseAfterPreAdmissionFailure() -> Bool {
    lock.lock()
    guard !beginSent, !admitted, gitFD >= 0 else {
      lock.unlock()
      return false
    }
    let fd = gitFD
    gitFD = -1
    cleanupProven = true
    lock.unlock()
    _ = flock(fd, LOCK_UN)
    _ = close(fd)
    xpc_connection_cancel(connection)
    return true
  }

  func releaseAfterCleanAdmissionRejection() -> Bool {
    lock.lock()
    guard beginSent, !admitted, gitFD >= 0 else {
      lock.unlock()
      return false
    }
    let fd = gitFD
    gitFD = -1
    lock.unlock()
    guard flock(fd, LOCK_UN) == 0 else {
      lock.lock()
      quarantined = true
      lock.unlock()
      return false
    }
    _ = close(fd)
    lock.lock()
    cleanupProven = true
    quarantined = false
    lock.unlock()
    helperExitSource?.cancel()
    xpc_connection_cancel(connection)
    return true
  }
}

private struct RetiredServiceChild {
  let slot: UnsafeMutablePointer<Int32>?
  let slotFD: Int32
  let slotLength: Int
  let timer: DispatchSourceTimer?
}

extension ServiceConnectionState {
  func matchesSession(_ message: xpc_object_t) -> Bool {
    lock.lock()
    defer { lock.unlock() }
    return helloed
      && sessionID == xpc_dictionary_get_uint64(message, "sessionId")
      && sessionNonceHigh == xpc_dictionary_get_uint64(message, "sessionNonceHigh")
      && sessionNonceLow == xpc_dictionary_get_uint64(message, "sessionNonceLow")
      && helperIncarnationHigh
        == xpc_dictionary_get_uint64(message, "helperIncarnationHigh")
      && helperIncarnationLow
        == xpc_dictionary_get_uint64(message, "helperIncarnationLow")
  }

  func admitSession(gitFD: Int32) -> Bool {
    lock.lock()
    guard helloed, sessionAdmissionStarted, !sessionAdmitted,
      !sessionEnding, !sessionQuarantined, !connectionInvalidated,
      gitReservationFD < 0, gitFD >= 0
    else {
      lock.unlock()
      return false
    }
    xpc_transaction_begin()
    manualTransactionActive = true
    gitReservationFD = gitFD
    sessionAdmitted = true
    sessionAdmissionStarted = false
    lock.unlock()
    return true
  }

  func beginSessionAdmission() -> Bool {
    lock.lock()
    defer { lock.unlock() }
    guard helloed, !sessionAdmissionStarted, !sessionAdmitted,
      !sessionEnding, !sessionQuarantined, !connectionInvalidated,
      gitReservationFD < 0
    else { return false }
    sessionAdmissionStarted = true
    return true
  }

  func rejectSessionAdmission() {
    lock.lock()
    if !sessionAdmitted { sessionAdmissionStarted = false }
    lock.unlock()
  }

  func hasPersistentSessionAuthority() -> Bool {
    lock.lock()
    defer { lock.unlock() }
    return sessionAdmitted || sessionEnding || sessionCleanupProven
  }

  func quarantinePersistentSession() {
    lock.lock()
    if sessionAdmitted || sessionEnding { sessionQuarantined = true }
    lock.unlock()
  }

  func validateSessionGitReservation() -> Bool {
    lock.lock()
    let fd = sessionAdmitted && !sessionEnding && !sessionQuarantined
      ? gitReservationFD : -1
    lock.unlock()
    return fd >= 0 && validateTransferredGitReservation(fd)
  }

  func prepareCleanSessionEnd() -> Int32? {
    lock.lock()
    defer { lock.unlock() }
    guard sessionAdmitted, !sessionEnding, !sessionQuarantined,
      !serviceQuarantined, childCleanupProven, !launchReserved,
      !ownerProtocolEngaged, !launched, !completed, requestID == 0,
      gitReservationFD >= 0, manualTransactionActive
    else { return nil }
    sessionEnding = true
    let fd = gitReservationFD
    gitReservationFD = -1
    return fd
  }

  func completeCleanSessionEnd() -> Bool {
    lock.lock()
    guard sessionAdmitted, sessionEnding,
      gitReservationFD < 0, childCleanupProven, !ownerProtocolEngaged,
      manualTransactionActive
    else {
      sessionQuarantined = true
      lock.unlock()
      return false
    }
    sessionAdmitted = false
    sessionEnding = false
    sessionCleanupProven = true
    sessionQuarantined = false
    manualTransactionActive = false
    lock.unlock()
    xpc_transaction_end()
    return true
  }

  func prepareOrphanedSessionEnd() -> Int32? {
    lock.lock()
    defer { lock.unlock() }
    guard sessionAdmitted, !sessionEnding, sessionQuarantined,
      connectionInvalidated, clientExitProven, childCleanupProven,
      !launchReserved, !ownerProtocolEngaged, !launched, !completed,
      requestID == 0, gitReservationFD >= 0, manualTransactionActive
    else { return nil }
    sessionEnding = true
    let fd = gitReservationFD
    gitReservationFD = -1
    return fd
  }

  func completeOrphanedSessionEnd() -> Bool {
    lock.lock()
    guard sessionAdmitted, sessionEnding, sessionQuarantined,
      connectionInvalidated, clientExitProven, gitReservationFD < 0,
      childCleanupProven, !ownerProtocolEngaged, manualTransactionActive
    else {
      lock.unlock()
      return false
    }
    sessionAdmitted = false
    sessionEnding = false
    sessionCleanupProven = true
    sessionQuarantined = false
    manualTransactionActive = false
    lock.unlock()
    xpc_transaction_end()
    return true
  }

  func sendRejectedChildAndReset(_ reply: xpc_object_t) -> Bool {
    lock.lock()
    let cleanWithoutOwner = launchReserved && !ownerProtocolEngaged && !launched
    let cleanWithProof = completed && ownerProtocolEngaged
      && pidSlot.map({ loadPIDSlotOwner($0) == .cleanupProven }) == true
    guard cleanWithoutOwner || cleanWithProof else {
      lock.unlock()
      return false
    }
    let retired = detachChildLocked()
    lock.unlock()
    releaseRetiredServiceChild(retired)
    xpc_connection_send_message(connection, reply)
    return true
  }

  func resetTerminalChildBeforeReply() -> TerminalChildResetDisposition {
    lock.lock()
    guard completed, let pidSlot,
      loadPIDSlotOwner(pidSlot) == .cleanupProven
    else {
      serviceQuarantined = true
      lock.unlock()
      return .failure
    }
    let pendingCancel = cancelRequest
    let retired = detachChildLocked()
    lock.unlock()
    releaseRetiredServiceChild(retired)
    return .success(pendingCancel)
  }

  func resetCompletedChildAfterProof() -> Bool {
    lock.lock()
    guard completed, ownerProtocolEngaged, let pidSlot,
      loadPIDSlotOwner(pidSlot) == .cleanupProven
    else {
      lock.unlock()
      return false
    }
    let retired = detachChildLocked()
    lock.unlock()
    releaseRetiredServiceChild(retired)
    return true
  }

  private func detachChildLocked() -> RetiredServiceChild {
    let retired = RetiredServiceChild(
      slot: pidSlot,
      slotFD: pidSlotFD,
      slotLength: pidSlotLength,
      timer: clientCleanupTimer
    )
    if completed {
      lastTerminalRequestID = requestID
      lastTerminalNonceHigh = nonceHigh
      lastTerminalNonceLow = nonceLow
      lastTerminalError = cleanupError
    }
    requestID = 0
    nonceHigh = 0
    nonceLow = 0
    pid = 0
    pidSlotFD = -1
    pidSlot = nil
    pidSlotLength = 0
    launchReserved = false
    ownerProtocolEngaged = false
    launched = false
    resumed = false
    terminating = false
    cleanupStarted = false
    reaperActive = false
    completed = false
    serviceQuarantined = false
    cleanupError = nil
    cancelRequest = nil
    clientCleanupTimer = nil
    childCleanupProven = true
    return retired
  }
}

private func releaseRetiredServiceChild(_ retired: RetiredServiceChild) {
  retired.timer?.cancel()
  if let slot = retired.slot, retired.slotLength > 0 {
    _ = munmap(UnsafeMutableRawPointer(slot), retired.slotLength)
  }
  if retired.slotFD >= 0 { _ = close(retired.slotFD) }
}

func setServiceSessionEnvelope(
  _ message: xpc_object_t,
  state: ServiceConnectionState
) {
  state.lock.lock()
  let sessionID = state.sessionID
  let nonceHigh = state.sessionNonceHigh
  let nonceLow = state.sessionNonceLow
  let incarnationHigh = state.helperIncarnationHigh
  let incarnationLow = state.helperIncarnationLow
  state.lock.unlock()
  xpc_dictionary_set_uint64(message, "sessionId", sessionID)
  xpc_dictionary_set_uint64(message, "sessionNonceHigh", nonceHigh)
  xpc_dictionary_set_uint64(message, "sessionNonceLow", nonceLow)
  xpc_dictionary_set_uint64(message, "helperIncarnationHigh", incarnationHigh)
  xpc_dictionary_set_uint64(message, "helperIncarnationLow", incarnationLow)
}
