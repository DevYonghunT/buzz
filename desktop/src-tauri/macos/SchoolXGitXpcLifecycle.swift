import Darwin
import Dispatch
import Foundation
import XPC

extension ServiceConnectionState {
  func reserveServiceReaper(pid: Int32) -> Bool {
    lock.lock()
    defer { lock.unlock() }
    guard launched, self.pid == pid, !reaperActive else { return false }
    reaperActive = true
    return true
  }

  func yieldServiceReaperToClient() -> Bool {
    lock.lock()
    defer { lock.unlock() }
    guard let pidSlot, loadPIDSlotOwner(pidSlot) == .clientAborting else {
      return false
    }
    reaperActive = false
    return true
  }

  func serviceReaperEnvelope(pid: Int32) -> (UInt64, UInt64, UInt64)? {
    lock.lock()
    defer { lock.unlock() }
    guard launched, self.pid == pid, requestID != 0, nonceHigh != 0, nonceLow != 0
    else { return nil }
    return (requestID, nonceHigh, nonceLow)
  }

  func armClientCleanupObservation() -> Bool {
    let timer = DispatchSource.makeTimerSource(
      queue: DispatchQueue.global(qos: .userInitiated)
    )
    timer.schedule(deadline: .now() + .milliseconds(100), repeating: .milliseconds(100))
    lock.lock()
    guard ownerProtocolEngaged, clientCleanupTimer == nil else {
      lock.unlock()
      timer.resume()
      timer.cancel()
      return false
    }
    let expectedRequestID = requestID
    let expectedNonceHigh = nonceHigh
    let expectedNonceLow = nonceLow
    clientCleanupTimer = timer
    lock.unlock()
    timer.setEventHandler { [weak self] in
      guard let self else { return }
      handleClientCleanupProgress(
        state: self,
        requestID: expectedRequestID,
        nonceHigh: expectedNonceHigh,
        nonceLow: expectedNonceLow
      )
    }
    timer.resume()
    return true
  }

  func matchesActiveChild(
    requestID: UInt64,
    nonceHigh: UInt64,
    nonceLow: UInt64
  ) -> Bool {
    lock.lock()
    defer { lock.unlock() }
    return ownerProtocolEngaged && self.requestID == requestID
      && self.nonceHigh == nonceHigh && self.nonceLow == nonceLow
  }

}

private func handleClientCleanupProgress(
  state: ServiceConnectionState,
  requestID: UInt64,
  nonceHigh: UInt64,
  nonceLow: UInt64
) {
  guard state.matchesActiveChild(
    requestID: requestID,
    nonceHigh: nonceHigh,
    nonceLow: nonceLow
  ) else { return }
  guard state.hasPersistentSessionAuthority() else { return }
  if state.acceptCleanupProof(error: nil) {
    _ = state.resetCompletedChildAfterProof()
    releaseOrphanedSessionAfterClientExit(state)
    return
  }
  guard state.connectionWasInvalidated() else { return }
  executePersistentCleanupDisposition(
    state.beginConnectionInvalidationCleanup(),
    state: state
  )
}

func handlePersistentServiceConnectionFailure(_ state: ServiceConnectionState) {
  let retainedSession = state.hasPersistentSessionAuthority()
  if retainedSession { state.quarantinePersistentSession() }
  executePersistentCleanupDisposition(
    state.beginConnectionInvalidationCleanup(),
    state: state
  )
  if !retainedSession { releaseServiceConnection(state) }
}

private func executePersistentCleanupDisposition(
  _ disposition: ServiceCleanupDisposition,
  state: ServiceConnectionState
) {
  switch disposition {
  case .kill(let pid):
    if pid > 1 {
      continueOrphanedServiceCleanup(state: state, pid: pid)
    } else if state.proveCleanup(error: nil) {
      _ = state.resetCompletedChildAfterProof()
    }
  case .cleanupProven:
    _ = state.acceptCleanupProof(error: nil)
    _ = state.resetCompletedChildAfterProof()
  case .clientTakeover:
    _ = state.armClientCleanupObservation()
  case .awaitCleanup, .noAuthority, .invalid:
    break
  }
  releaseOrphanedSessionAfterClientExit(state)
}

private func releaseOrphanedSessionAfterClientExit(
  _ state: ServiceConnectionState
) {
  guard let fd = state.prepareOrphanedSessionEnd() else { return }
  guard close(fd) == 0, state.completeOrphanedSessionEnd() else {
    state.quarantinePersistentSession()
    return
  }
  releaseGlobalServiceOperation(state)
  releaseServiceConnection(state)
}

func continueOrphanedServiceCleanup(state: ServiceConnectionState, pid: Int32) {
  killProcessGroup(pid)
  guard let envelope = state.serviceReaperEnvelope(pid: pid) else { return }
  startServiceReaper(
    state: state,
    requestID: envelope.0,
    nonceHigh: envelope.1,
    nonceLow: envelope.2,
    pid: pid
  )
}

func killAndProveServiceSpawnFailure(
  state: ServiceConnectionState,
  pid: Int32
) -> Bool {
  switch state.beginSpawnFailureCleanup(pid: pid) {
  case .kill(let cleanupPID):
    killProcessGroup(cleanupPID)
  case .awaitCleanup:
    break
  case .cleanupProven:
    return state.acceptCleanupProof(error: nil)
  case .clientTakeover, .noAuthority, .invalid:
    return false
  }
  guard state.markPIDSlotTerminal() else { return false }
  var rawStatus: Int32 = 0
  while true {
    let result = waitpid(pid, &rawStatus, 0)
    if result == pid { break }
    if result < 0, errno == EINTR { continue }
    return false
  }
  return waitForProcessGroupExit(pid) && state.proveCleanup(error: nil)
}

func startServiceReaper(
  state: ServiceConnectionState,
  requestID: UInt64,
  nonceHigh: UInt64,
  nonceLow: UInt64,
  pid: Int32
) {
  guard state.reserveServiceReaper(pid: pid) else { return }
  DispatchQueue.global(qos: .userInitiated).async {
    reapGitProcessGroup(
      state: state,
      requestID: requestID,
      nonceHigh: nonceHigh,
      nonceLow: nonceLow,
      pid: pid
    )
  }
}

func quarantineServiceCleanup(
  state: ServiceConnectionState,
  requestID: UInt64,
  nonceHigh: UInt64,
  nonceLow: UInt64,
  diagnostic: String
) {
  state.quarantineServiceAuthority()
  retainServiceConnection(state)
  sendServiceQuarantined(
    state: state,
    request: nil,
    kind: "cleanupQuarantined",
    requestID: requestID,
    nonceHigh: nonceHigh,
    nonceLow: nonceLow,
    diagnostic: diagnostic
  )
}

func sendFinished(
  state: ServiceConnectionState,
  requestID: UInt64,
  nonceHigh: UInt64,
  nonceLow: UInt64,
  rawStatus: Int32,
  error: String?
) {
  let message = xpc_dictionary_create(nil, nil, 0)
  xpc_dictionary_set_string(message, "kind", "finished")
  setServiceSessionEnvelope(message, state: state)
  xpc_dictionary_set_uint64(message, "requestId", requestID)
  xpc_dictionary_set_uint64(message, "nonceHigh", nonceHigh)
  xpc_dictionary_set_uint64(message, "nonceLow", nonceLow)
  xpc_dictionary_set_int64(message, "rawStatus", Int64(rawStatus))
  if let error { xpc_dictionary_set_string(message, "error", error) }
  let pendingCancel: xpc_object_t?
  switch state.resetTerminalChildBeforeReply() {
  case .success(let request):
    pendingCancel = request
  case .failure:
    return
  }
  let cancelReply = pendingCancel.flatMap { request in
    makeCancelAcknowledgment(
      state: state,
      request: request,
      requestID: requestID,
      nonceHigh: nonceHigh,
      nonceLow: nonceLow,
      error: error
    )
  }
  releaseOrphanedSessionAfterClientExit(state)
  if let cancelReply { xpc_connection_send_message(state.connection, cancelReply) }
  xpc_connection_send_message(state.connection, message)
}

func sendCancelAcknowledgment(
  state: ServiceConnectionState,
  request: xpc_object_t,
  requestID: UInt64,
  nonceHigh: UInt64,
  nonceLow: UInt64,
  error: String?
) {
  guard let reply = makeCancelAcknowledgment(
    state: state,
    request: request,
    requestID: requestID,
    nonceHigh: nonceHigh,
    nonceLow: nonceLow,
    error: error
  ) else { return }
  xpc_connection_send_message(state.connection, reply)
}

private func makeCancelAcknowledgment(
  state: ServiceConnectionState,
  request: xpc_object_t,
  requestID: UInt64,
  nonceHigh: UInt64,
  nonceLow: UInt64,
  error: String?
) -> xpc_object_t? {
  guard let reply = xpc_dictionary_create_reply(request) else { return nil }
  xpc_dictionary_set_string(reply, "kind", "cancelAck")
  setServiceSessionEnvelope(reply, state: state)
  xpc_dictionary_set_uint64(reply, "requestId", requestID)
  xpc_dictionary_set_uint64(reply, "nonceHigh", nonceHigh)
  xpc_dictionary_set_uint64(reply, "nonceLow", nonceLow)
  xpc_dictionary_set_bool(reply, "ok", error == nil)
  xpc_dictionary_set_bool(reply, "childCleanupProven", true)
  xpc_dictionary_set_bool(reply, "childAuthorityRetained", false)
  if let error { xpc_dictionary_set_string(reply, "error", error) }
  return reply
}

func retainServiceConnection(_ state: ServiceConnectionState) {
  serviceConnectionsLock.lock()
  serviceConnections[state.identity] = state
  serviceConnectionsLock.unlock()
}

func releaseServiceConnection(_ state: ServiceConnectionState) {
  guard state.mayReleaseConnectionState() else { return }
  serviceConnectionsLock.lock()
  if serviceConnections[state.identity] === state {
    serviceConnections.removeValue(forKey: state.identity)
  }
  serviceConnectionsLock.unlock()
}

func reserveGlobalServiceOperation(_ state: ServiceConnectionState) -> Bool {
  activeServiceOperationLock.lock()
  defer { activeServiceOperationLock.unlock() }
  guard activeServiceOperation == nil else { return false }
  activeServiceOperation = state.identity
  return true
}

func releaseGlobalServiceOperation(_ state: ServiceConnectionState) {
  activeServiceOperationLock.lock()
  if activeServiceOperation == state.identity {
    activeServiceOperation = nil
  }
  activeServiceOperationLock.unlock()
}

func handleAuthenticatedClientExit(state: ServiceConnectionState) {
  let disposition = state.authenticatedClientDidExit()
  guard state.hasPersistentSessionAuthority() else { return }
  executePersistentCleanupDisposition(disposition, state: state)
}

func sendServiceError(_ request: xpc_object_t, diagnostic: String) {
  guard let reply = xpc_dictionary_create_reply(request) else { return }
  xpc_dictionary_set_string(reply, "kind", "error")
  xpc_dictionary_set_string(reply, "error", diagnostic)
  if let connection = xpc_dictionary_get_remote_connection(request) {
    xpc_connection_send_message(connection, reply)
  }
}

@discardableResult
func sendLaunchRejected(
  state: ServiceConnectionState,
  request: xpc_object_t,
  requestID: UInt64,
  nonceHigh: UInt64,
  nonceLow: UInt64,
  diagnostic: String,
  resetChild: Bool = false
) -> Bool {
  guard let reply = xpc_dictionary_create_reply(request) else { return false }
  xpc_dictionary_set_string(reply, "kind", "launchRejected")
  setServiceSessionEnvelope(reply, state: state)
  xpc_dictionary_set_uint64(reply, "requestId", requestID)
  xpc_dictionary_set_uint64(reply, "nonceHigh", nonceHigh)
  xpc_dictionary_set_uint64(reply, "nonceLow", nonceLow)
  xpc_dictionary_set_string(reply, "error", diagnostic)
  xpc_dictionary_set_bool(reply, "childCleanupProven", true)
  xpc_dictionary_set_bool(reply, "childAuthorityRetained", false)
  if resetChild { return state.sendRejectedChildAndReset(reply) }
  xpc_connection_send_message(state.connection, reply)
  return true
}

func sendServiceQuarantined(
  state: ServiceConnectionState,
  request: xpc_object_t?,
  kind: String,
  requestID: UInt64,
  nonceHigh: UInt64,
  nonceLow: UInt64,
  diagnostic: String
) {
  let reply: xpc_object_t
  if let request {
    guard let response = xpc_dictionary_create_reply(request) else { return }
    reply = response
  } else {
    reply = xpc_dictionary_create(nil, nil, 0)
  }
  xpc_dictionary_set_string(reply, "kind", kind)
  setServiceSessionEnvelope(reply, state: state)
  xpc_dictionary_set_uint64(reply, "requestId", requestID)
  xpc_dictionary_set_uint64(reply, "nonceHigh", nonceHigh)
  xpc_dictionary_set_uint64(reply, "nonceLow", nonceLow)
  xpc_dictionary_set_int64(reply, "helperPid", Int64(getpid()))
  xpc_dictionary_set_int64(reply, "helperSid", Int64(getsid(0)))
  xpc_dictionary_set_string(reply, "error", diagnostic)
  xpc_dictionary_set_bool(reply, "childCleanupProven", false)
  xpc_dictionary_set_bool(reply, "childAuthorityRetained", true)
  xpc_connection_send_message(state.connection, reply)
}

func serviceSessionHello(state: ServiceConnectionState, message: xpc_object_t) {
  let sessionID = xpc_dictionary_get_uint64(message, "sessionId")
  let nonceHigh = xpc_dictionary_get_uint64(message, "sessionNonceHigh")
  let nonceLow = xpc_dictionary_get_uint64(message, "sessionNonceLow")
  let helperPID = getpid()
  let helperPGID = getpgid(0)
  let helperSID = getsid(0)
  guard xpc_dictionary_get_uint64(message, "protocolVersion") == protocolVersion,
    helperPID > 1, helperPGID == helperPID, helperSID > 0,
    state.installSessionHello(
      sessionID: sessionID,
      sessionNonceHigh: nonceHigh,
      sessionNonceLow: nonceLow,
      clientPID: xpc_connection_get_pid(state.connection)
    ), state.armClientExitObservation(),
    let reply = xpc_dictionary_create_reply(message)
  else {
    sendServiceError(message, diagnostic: "typed Git session handshake authority did not match")
    return
  }
  xpc_dictionary_set_string(reply, "kind", "sessionHelloAck")
  setServiceSessionEnvelope(reply, state: state)
  xpc_dictionary_set_uint64(reply, "protocolVersion", protocolVersion)
  xpc_dictionary_set_int64(reply, "helperPid", Int64(helperPID))
  xpc_dictionary_set_int64(reply, "helperPgid", Int64(helperPGID))
  xpc_dictionary_set_int64(reply, "helperSid", Int64(helperSID))
  xpc_connection_send_message(state.connection, reply)
}

func serviceSessionBegin(state: ServiceConnectionState, message: xpc_object_t) {
  guard state.matchesSession(message) else {
    sendServiceError(message, diagnostic: "typed Git session admission authority did not match")
    return
  }
  guard state.beginSessionAdmission() else {
      sendServiceSessionResult(
        state: state, request: message, kind: "sessionBegan", ok: false,
        proven: false, retained: true, error: "typed Git session admission was replayed"
      )
      return
  }
  let fd = xpc_dictionary_dup_fd(message, "gitFd")
  var diagnostic: String?
  if fd < 0 || !validateTransferredGitReservation(fd) {
    diagnostic = "fixed system Git reservation was invalid"
  } else if !reserveGlobalServiceOperation(state) {
    diagnostic = "another typed Git authority session is still active"
  } else if !state.admitSession(gitFD: fd) {
    releaseGlobalServiceOperation(state)
    diagnostic = "typed Git session lifetime authority was unavailable"
  }
  if let diagnostic {
    state.rejectSessionAdmission()
    if fd >= 0 { _ = close(fd) }
    sendServiceSessionResult(
      state: state, request: message, kind: "sessionBegan", ok: false,
      proven: true, retained: false, error: diagnostic
    )
    return
  }
  sendServiceSessionResult(
    state: state, request: message, kind: "sessionBegan", ok: true,
    proven: false, retained: true, error: nil
  )
}

func serviceSessionEnd(state: ServiceConnectionState, message: xpc_object_t) {
  guard state.matchesSession(message), let fd = state.prepareCleanSessionEnd() else {
    sendServiceSessionResult(
      state: state, request: message, kind: "sessionEnded", ok: false,
      proven: false, retained: true,
      error: "typed Git session still retains child or quarantine authority"
    )
    return
  }
  guard close(fd) == 0, state.completeCleanSessionEnd() else {
    state.quarantinePersistentSession()
    sendServiceSessionResult(
      state: state, request: message, kind: "sessionEnded", ok: false,
      proven: false, retained: true,
      error: "typed Git session reservation release was ambiguous"
    )
    return
  }
  releaseGlobalServiceOperation(state)
  sendServiceSessionResult(
    state: state, request: message, kind: "sessionEnded", ok: true,
    proven: true, retained: false, error: nil
  )
  releaseServiceConnection(state)
}

private func sendServiceSessionResult(
  state: ServiceConnectionState,
  request: xpc_object_t,
  kind: String,
  ok: Bool,
  proven: Bool,
  retained: Bool,
  error: String?
) {
  guard let reply = xpc_dictionary_create_reply(request) else { return }
  xpc_dictionary_set_string(reply, "kind", kind)
  setServiceSessionEnvelope(reply, state: state)
  xpc_dictionary_set_bool(reply, "ok", ok)
  xpc_dictionary_set_bool(reply, "sessionCleanupProven", proven)
  xpc_dictionary_set_bool(reply, "sessionAuthorityRetained", retained)
  if let error { xpc_dictionary_set_string(reply, "error", error) }
  xpc_connection_send_message(state.connection, reply)
}
