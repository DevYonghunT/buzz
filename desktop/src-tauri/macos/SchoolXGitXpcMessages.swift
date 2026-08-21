import Darwin
import Foundation
import XPC

func parseStartedReply(_ reply: xpc_object_t, operation: ClientOperation) -> Int32? {
  guard xpc_get_type(reply) == XPC_TYPE_DICTIONARY,
    dictionaryString(reply, key: "kind") == "started",
    messageMatchesSession(reply, operation: operation),
    xpc_dictionary_get_uint64(reply, "requestId") == operation.requestID,
    xpc_dictionary_get_uint64(reply, "nonceHigh") == operation.nonceHigh,
    xpc_dictionary_get_uint64(reply, "nonceLow") == operation.nonceLow
  else { return nil }
  let pid64 = xpc_dictionary_get_int64(reply, "pid")
  let helperPID64 = xpc_dictionary_get_int64(reply, "helperPid")
  let helperSID64 = xpc_dictionary_get_int64(reply, "helperSid")
  guard pid64 > 0, pid64 <= Int64(Int32.max), helperPID64 > 0,
    helperPID64 <= Int64(Int32.max), helperSID64 > 0,
    helperSID64 <= Int64(Int32.max),
    xpc_connection_get_pid(operation.connection) == Int32(helperPID64),
    operation.matchesHelper(pid: Int32(helperPID64), sid: Int32(helperSID64)),
    operation.installPID(Int32(pid64)),
    getpgid(Int32(pid64)) == Int32(pid64),
    getsid(Int32(helperPID64)) == Int32(helperSID64),
    getsid(Int32(pid64)) == Int32(helperSID64)
  else { return nil }
  return Int32(pid64)
}

func parseLaunchRejection(
  _ reply: xpc_object_t,
  operation: ClientOperation
) -> String? {
  guard xpc_get_type(reply) == XPC_TYPE_DICTIONARY,
    dictionaryString(reply, key: "kind") == "launchRejected",
    messageMatchesSession(reply, operation: operation),
    xpc_dictionary_get_uint64(reply, "requestId") == operation.requestID,
    xpc_dictionary_get_uint64(reply, "nonceHigh") == operation.nonceHigh,
    xpc_dictionary_get_uint64(reply, "nonceLow") == operation.nonceLow,
    let diagnostic = dictionaryString(reply, key: "error"),
    !diagnostic.isEmpty
  else { return nil }
  return diagnostic
}

func parseServiceQuarantine(
  _ message: xpc_object_t,
  kind: String,
  operation: ClientOperation
) -> String? {
  guard xpc_get_type(message) == XPC_TYPE_DICTIONARY,
    dictionaryString(message, key: "kind") == kind,
    messageMatchesSession(message, operation: operation),
    xpc_dictionary_get_uint64(message, "requestId") == operation.requestID,
    xpc_dictionary_get_uint64(message, "nonceHigh") == operation.nonceHigh,
    xpc_dictionary_get_uint64(message, "nonceLow") == operation.nonceLow,
    let diagnostic = dictionaryString(message, key: "error"),
    !diagnostic.isEmpty
  else { return nil }
  let helperPID = xpc_dictionary_get_int64(message, "helperPid")
  let helperSID = xpc_dictionary_get_int64(message, "helperSid")
  guard helperPID > 1, helperPID <= Int64(Int32.max),
    helperSID > 0, helperSID <= Int64(Int32.max),
    xpc_connection_get_pid(operation.connection) == Int32(helperPID),
    operation.matchesHelper(pid: Int32(helperPID), sid: Int32(helperSID))
  else { return nil }
  return diagnostic
}

func handleClientEvent(operation: ClientOperation, event: xpc_object_t) {
  if xpc_get_type(event) == XPC_TYPE_ERROR {
    operation.fail("signed XPC helper connection failed: \(xpcDescription(event))")
    return
  }
  if let diagnostic = parseServiceQuarantine(
    event,
    kind: "cleanupQuarantined",
    operation: operation
  ) {
    operation.quarantineService(diagnostic, permanent: true)
    return
  }
  guard xpc_get_type(event) == XPC_TYPE_DICTIONARY,
    dictionaryString(event, key: "kind") == "finished",
    messageMatchesSession(event, operation: operation),
    xpc_dictionary_get_uint64(event, "requestId") == operation.requestID,
    xpc_dictionary_get_uint64(event, "nonceHigh") == operation.nonceHigh,
    xpc_dictionary_get_uint64(event, "nonceLow") == operation.nonceLow
  else {
    operation.fail("signed XPC helper sent an invalid event")
    return
  }
  let diagnostic = dictionaryString(event, key: "error")
  let raw = xpc_dictionary_get_int64(event, "rawStatus")
  guard raw >= Int64(Int32.min), raw <= Int64(Int32.max),
    operation.acceptServiceFinished(
      rawStatus: Int32(raw),
      error: diagnostic.flatMap { $0.isEmpty ? nil : $0 }
    )
  else {
    operation.fail("signed XPC helper Finished lacked cleanup-owner proof")
    return
  }
  if operation.shouldRemoveAfterServiceFinished() {
    removeClientOperation(operation.requestID, cancel: true)
  }
}

func schoolx_git_xpc_poll(request_id: UInt64) -> RustString {
  guard let operation = lookupClientOperation(request_id) else {
    return encodePollFailure("unknown XPC Git request")
  }
  switch operation.poll() {
  case .pending:
    return RustString("{\"state\":\"pending\"}")
  case .finished(let rawStatus):
    operation.retireExposedHandle()
    removeClientOperation(request_id, cancel: true)
    return RustString("{\"state\":\"finished\",\"rawStatus\":\(rawStatus)}")
  case .failed(let diagnostic):
    operation.retireExposedHandle()
    removeClientOperation(request_id, cancel: true)
    return encodePollFailure(diagnostic, operation)
  }
}

func schoolx_git_xpc_cancel(request_id: UInt64) -> RustString {
  guard let operation = lookupClientOperation(request_id) else {
    return encodeChildFailure(
      "unknown XPC Git request",
      cleanupProvenWithoutOperation: false
    )
  }
  switch operation.poll() {
  case .finished(_):
    operation.retireExposedHandle()
    removeClientOperation(request_id, cancel: true)
    return encodeRustString(
      EncodedChildResult(
        ok: true,
        childCleanupProven: true,
        childAuthorityRetained: false
      ))
  case .failed(let diagnostic):
    operation.retireExposedHandle()
    removeClientOperation(request_id, cancel: true)
    return encodeChildFailure(diagnostic, operation)
  case .pending:
    break
  }
  operation.retireExposedHandle()
  if let diagnostic = requestServiceCancellation(operation, requestID: request_id) {
    _ = operation.abortLaunch(diagnostic)
    operation.recoverAfterCancelFailureIfHelperExited()
    return encodeChildFailure(diagnostic, operation)
  }
  removeClientOperation(request_id, cancel: true)
  return encodeRustString(
    EncodedChildResult(
      ok: true,
      childCleanupProven: true,
      childAuthorityRetained: false
    ))
}

func requestServiceCancellation(
  _ operation: ClientOperation,
  requestID: UInt64
) -> String? {
  let cancel = xpc_dictionary_create(nil, nil, 0)
  xpc_dictionary_set_string(cancel, "kind", "cancel")
  setSessionEnvelope(cancel, operation: operation)
  xpc_dictionary_set_uint64(cancel, "requestId", requestID)
  xpc_dictionary_set_uint64(cancel, "nonceHigh", operation.nonceHigh)
  xpc_dictionary_set_uint64(cancel, "nonceLow", operation.nonceLow)
  let replyBox = ReplyBox()
  xpc_connection_send_message_with_reply(
    operation.connection,
    cancel,
    operation.queue
  ) { reply in
    replyBox.store(reply)
  }
  guard replyBox.wait(timeout: cancelTimeout), let reply = replyBox.take(),
    xpc_get_type(reply) == XPC_TYPE_DICTIONARY,
    dictionaryString(reply, key: "kind") == "cancelAck",
    messageMatchesSession(reply, operation: operation),
    xpc_dictionary_get_uint64(reply, "requestId") == requestID,
    xpc_dictionary_get_uint64(reply, "nonceHigh") == operation.nonceHigh,
    xpc_dictionary_get_uint64(reply, "nonceLow") == operation.nonceLow,
    xpc_dictionary_get_bool(reply, "childCleanupProven"),
    !xpc_dictionary_get_bool(reply, "childAuthorityRetained")
  else {
    return "signed XPC helper cancellation timed out"
  }
  guard operation.acceptServiceCleanupProof() else {
    operation.quarantineService(
      "signed XPC helper cancellation lacked cleanup-owner proof",
      permanent: true
    )
    return "signed XPC helper cancellation lacked cleanup proof"
  }
  if !xpc_dictionary_get_bool(reply, "ok") {
    let diagnostic = dictionaryString(reply, key: "error")
      ?? "signed XPC helper could not prove cancellation cleanup"
    return diagnostic
  }
  return nil
}
