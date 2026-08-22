import Darwin
import Dispatch
import Foundation
import Security
import XPC

let schoolXAppIdentifier = "io.github.schoolx520.app"
let schoolXGitServiceIdentifier =
  "io.github.schoolx520.app.schoolx-code-git"
let maximumPayloadBytes = 512 * 1024
// Version 3 binds sequential children to one authenticated reservation session.
// Exact hello matching prevents older helpers from spawning outside that lease.
let protocolVersion: UInt64 = 3
let helloTimeout = DispatchTimeInterval.seconds(5)
let launchTimeout = DispatchTimeInterval.seconds(10)
let resumeTimeout = DispatchTimeInterval.seconds(5)
let cancelTimeout = DispatchTimeInterval.seconds(5)
private let pidSlotOwnerOffset = MemoryLayout<Int32>.size
private let pidSlotNonceHighOffset: off_t = 8
private let pidSlotNonceLowOffset: off_t = 16

enum PIDSlotOwnerState: Int32 {
  case unclaimed = 0
  case serviceLaunch = 1
  case serviceCleaning = 2
  case clientAborting = 3
  case cleanupProven = 4
}

enum ServiceLaunchOwnerClaim {
  case acquired
  case clientAborting
  case serviceCleaning
  case cleanupProven
  case invalid
}

enum ServiceCleanupOwnerClaim {
  case acquired
  case alreadyCleaning
  case clientAborting
  case cleanupProven
  case invalid
}

enum ClientAbortOwnerClaim {
  case acquired(noChildCouldHaveSpawned: Bool)
  case alreadyOwned
  case serviceCleaning
  case cleanupProven
  case invalid
}

struct EncodedResult: Encodable {
  let ok: Bool
  var pid: UInt32 = 0
  var error: String = ""
}

final class ReplyBox {
  private let semaphore = DispatchSemaphore(value: 0)
  private let lock = NSLock()
  private var reply: xpc_object_t?

  func store(_ value: xpc_object_t) {
    lock.lock()
    reply = value
    lock.unlock()
    semaphore.signal()
  }

  func wait(timeout: DispatchTimeInterval) -> Bool {
    semaphore.wait(timeout: .now() + timeout) == .success
  }

  func take() -> xpc_object_t? {
    lock.lock()
    defer { lock.unlock() }
    return reply
  }
}

enum PIDSlotResult {
  case success(Int32, UnsafeMutablePointer<Int32>, Int)
  case failure(String)
}

func createPIDSlot(nonceHigh: UInt64, nonceLow: UInt64) -> PIDSlotResult {
  var template = Array(
    (NSTemporaryDirectory() + "schoolx-code-git.XXXXXX").utf8CString)
  let fd = template.withUnsafeMutableBufferPointer { buffer -> Int32 in
    guard let base = buffer.baseAddress else { return -1 }
    return mkstemp(base)
  }
  guard fd >= 0 else { return .failure("failed to create XPC PID slot: \(errnoDiagnostic())") }
  let unlinkResult = template.withUnsafeBufferPointer { buffer -> Int32 in
    guard let base = buffer.baseAddress else { return -1 }
    return unlink(base)
  }
  guard unlinkResult == 0 else {
    let diagnostic = errnoDiagnostic()
    _ = close(fd)
    return .failure("failed to unlink XPC PID slot: \(diagnostic)")
  }
  guard fcntl(fd, F_SETFD, FD_CLOEXEC) == 0 else {
    let diagnostic = errnoDiagnostic()
    _ = close(fd)
    return .failure("failed to protect XPC PID slot: \(diagnostic)")
  }
  let length = Int(sysconf(_SC_PAGESIZE))
  guard length >= pidSlotOwnerOffset + MemoryLayout<Int32>.size,
    ftruncate(fd, off_t(length)) == 0
  else {
    let diagnostic = errnoDiagnostic()
    _ = close(fd)
    return .failure("failed to size XPC PID slot: \(diagnostic)")
  }
  guard writeSlotUInt64(fd: fd, value: nonceHigh, offset: pidSlotNonceHighOffset),
    writeSlotUInt64(fd: fd, value: nonceLow, offset: pidSlotNonceLowOffset)
  else {
    let diagnostic = errnoDiagnostic()
    _ = close(fd)
    return .failure("failed to bind XPC PID slot nonce: \(diagnostic)")
  }
  guard let slot = mapPIDSlot(fd: fd, length: length) else {
    let diagnostic = errnoDiagnostic()
    _ = close(fd)
    return .failure("failed to map XPC PID slot: \(diagnostic)")
  }
  slot.pointee = 0
  guard loadPIDSlotOwner(slot) == .unclaimed else {
    _ = munmap(UnsafeMutableRawPointer(slot), length)
    _ = close(fd)
    return .failure("failed to initialize XPC cleanup owner authority")
  }
  return .success(fd, slot, length)
}

func mapPIDSlot(fd: Int32, length: Int) -> UnsafeMutablePointer<Int32>? {
  let mapping = mmap(nil, length, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0)
  guard mapping != MAP_FAILED, let mapping else { return nil }
  return mapping.assumingMemoryBound(to: Int32.self)
}

func readPIDSlot(fd: Int32) -> Int32 {
  readPIDSlotExact(fd: fd) ?? 0
}

func readPIDSlotExact(fd: Int32) -> Int32? {
  var value: Int32 = 0
  let count = pread(fd, &value, MemoryLayout<Int32>.size, 0)
  return count == MemoryLayout<Int32>.size ? value : nil
}

func loadPIDSlotOwner(_ slot: UnsafeMutablePointer<Int32>) -> PIDSlotOwnerState? {
  let raw = OSAtomicAdd32Barrier(0, pidSlotOwnerPointer(slot))
  return PIDSlotOwnerState(rawValue: raw)
}

func claimServiceLaunchOwner(_ slot: UnsafeMutablePointer<Int32>)
  -> ServiceLaunchOwnerClaim
{
  if compareAndSwapPIDSlotOwner(slot, from: .unclaimed, to: .serviceLaunch) {
    return .acquired
  }
  switch loadPIDSlotOwner(slot) {
  case .clientAborting:
    return .clientAborting
  case .serviceCleaning:
    return .serviceCleaning
  case .cleanupProven:
    return .cleanupProven
  default:
    return .invalid
  }
}

func claimOrphanedClientCleanupOwner(_ slot: UnsafeMutablePointer<Int32>)
  -> ServiceCleanupOwnerClaim
{
  if compareAndSwapPIDSlotOwner(slot, from: .clientAborting, to: .serviceCleaning) {
    return .acquired
  }
  return claimServiceCleanupOwner(slot)
}

func claimServiceCleanupOwner(_ slot: UnsafeMutablePointer<Int32>)
  -> ServiceCleanupOwnerClaim
{
  if compareAndSwapPIDSlotOwner(slot, from: .serviceLaunch, to: .serviceCleaning) {
    return .acquired
  }
  switch loadPIDSlotOwner(slot) {
  case .serviceCleaning:
    return .alreadyCleaning
  case .clientAborting:
    return .clientAborting
  case .cleanupProven:
    return .cleanupProven
  default:
    return .invalid
  }
}

func claimServiceRuntimeOwner(_ slot: UnsafeMutablePointer<Int32>) -> Bool {
  compareAndSwapPIDSlotOwner(slot, from: .serviceLaunch, to: .serviceCleaning)
}

func claimClientAbortOwner(_ slot: UnsafeMutablePointer<Int32>) -> ClientAbortOwnerClaim {
  while true {
    switch loadPIDSlotOwner(slot) {
    case .unclaimed:
      if compareAndSwapPIDSlotOwner(slot, from: .unclaimed, to: .clientAborting) {
        return .acquired(noChildCouldHaveSpawned: true)
      }
    case .serviceLaunch:
      if compareAndSwapPIDSlotOwner(slot, from: .serviceLaunch, to: .clientAborting) {
        return .acquired(noChildCouldHaveSpawned: true)
      }
    case .clientAborting:
      return .alreadyOwned
    case .serviceCleaning:
      return .serviceCleaning
    case .cleanupProven:
      return .cleanupProven
    case nil:
      return .invalid
    }
  }
}

func claimOrphanedServiceCleanupOwner(_ slot: UnsafeMutablePointer<Int32>)
  -> ClientAbortOwnerClaim
{
  if compareAndSwapPIDSlotOwner(slot, from: .serviceCleaning, to: .clientAborting) {
    return .acquired(noChildCouldHaveSpawned: false)
  }
  switch loadPIDSlotOwner(slot) {
  case .clientAborting:
    return .alreadyOwned
  case .serviceCleaning:
    return .serviceCleaning
  case .cleanupProven:
    return .cleanupProven
  default:
    return .invalid
  }
}

func proveServiceCleanupOwner(_ slot: UnsafeMutablePointer<Int32>) -> Bool {
  compareAndSwapPIDSlotOwner(slot, from: .serviceCleaning, to: .cleanupProven)
}

func proveClientCleanupOwner(_ slot: UnsafeMutablePointer<Int32>) -> Bool {
  compareAndSwapPIDSlotOwner(slot, from: .clientAborting, to: .cleanupProven)
}

private func pidSlotOwnerPointer(_ slot: UnsafeMutablePointer<Int32>)
  -> UnsafeMutablePointer<Int32>
{
  UnsafeMutableRawPointer(slot)
    .advanced(by: pidSlotOwnerOffset)
    .assumingMemoryBound(to: Int32.self)
}

private func compareAndSwapPIDSlotOwner(
  _ slot: UnsafeMutablePointer<Int32>,
  from: PIDSlotOwnerState,
  to: PIDSlotOwnerState
) -> Bool {
  OSAtomicCompareAndSwap32Barrier(from.rawValue, to.rawValue, pidSlotOwnerPointer(slot))
}

private func writeSlotUInt64(fd: Int32, value: UInt64, offset: off_t) -> Bool {
  var value = value
  return pwrite(fd, &value, MemoryLayout<UInt64>.size, offset)
    == MemoryLayout<UInt64>.size
}

private func readSlotUInt64(fd: Int32, offset: off_t) -> UInt64? {
  var value: UInt64 = 0
  guard pread(fd, &value, MemoryLayout<UInt64>.size, offset)
    == MemoryLayout<UInt64>.size
  else { return nil }
  return value
}

func validatePIDSlot(
  fd: Int32,
  length: Int,
  nonceHigh: UInt64,
  nonceLow: UInt64
) -> Bool {
  var value = stat()
  let flags = fcntl(fd, F_GETFL)
  guard fstat(fd, &value) == 0,
    (UInt32(value.st_mode) & UInt32(S_IFMT)) == UInt32(S_IFREG),
    value.st_size == off_t(length),
    value.st_nlink == 0,
    value.st_uid == geteuid(),
    flags >= 0,
    (flags & O_ACCMODE) == O_RDWR,
    readSlotUInt64(fd: fd, offset: pidSlotNonceHighOffset) == nonceHigh,
    readSlotUInt64(fd: fd, offset: pidSlotNonceLowOffset) == nonceLow
  else { return false }
  return true
}

func randomNonceComponent() -> UInt64 {
  var value: UInt64 = 0
  repeat {
    value = UInt64.random(in: UInt64.min...UInt64.max)
  } while value == 0
  return value
}

func capabilityDiagnostic() -> String? {
  guard #available(macOS 12.0, *) else {
    return "SchoolX Code Git requires macOS 12 or newer signed-helper support"
  }
  guard let identity = currentSigningIdentity(),
    identity.identifier == schoolXAppIdentifier,
    validTeamIdentifier(identity.teamIdentifier)
  else {
    return "SchoolX Code Git requires a Developer ID signed SchoolX application"
  }
  let service = Bundle.main.bundleURL
    .appendingPathComponent("Contents", isDirectory: true)
    .appendingPathComponent("XPCServices", isDirectory: true)
    .appendingPathComponent("\(schoolXGitServiceIdentifier).xpc", isDirectory: true)
  guard FileManager.default.fileExists(atPath: service.path) else {
    return "SchoolX Code Git signed helper is not embedded in this application"
  }
  guard validateStaticDeveloperIDCode(
    at: service,
    identifier: schoolXGitServiceIdentifier,
    teamIdentifier: identity.teamIdentifier
  ) else {
    return "SchoolX Code Git embedded helper failed Developer ID validation"
  }
  return nil
}

func dictionaryString(_ dictionary: xpc_object_t, key: String) -> String? {
  guard let pointer = xpc_dictionary_get_string(dictionary, key) else { return nil }
  return String(cString: pointer)
}

func encodeRustString<T: Encodable>(_ value: T) -> RustString {
  guard let data = try? JSONEncoder().encode(value),
    let encoded = String(data: data, encoding: .utf8)
  else { return RustString("{\"ok\":false,\"error\":\"failed to encode XPC response\"}") }
  return RustString(encoded)
}

func encodePollFailure(
  _ diagnostic: String,
  _ operation: ClientOperation? = nil
) -> RustString {
  let disposition = operation?.childAuthorityDisposition()
    ?? (proven: false, retained: true)
  return encodeRustString(
    EncodedPollFailure(
      error: diagnostic,
      childCleanupProven: disposition.proven,
      childAuthorityRetained: disposition.retained
    ))
}

func xpcDescription(_ object: xpc_object_t) -> String {
  let pointer = xpc_copy_description(object)
  defer { free(pointer) }
  return String(cString: pointer)
}

func validateRootTrustedFilesystemObject(_ path: String) -> Bool {
  guard geteuid() != 0 else { return false }
  errno = 0
  guard access(path, W_OK) != 0,
    errno == EACCES || errno == EPERM || errno == EROFS
  else { return false }
  errno = 0
  if let acl = acl_get_file(path, ACL_TYPE_EXTENDED) {
    _ = acl_free(UnsafeMutableRawPointer(acl))
    return false
  }
  return errno == ENOENT
}

func validateGitReservationFD(_ fd: Int32) -> Bool {
  guard fd >= 0 else { return false }
  var descriptor = stat()
  var path = stat()
  let flags = fcntl(fd, F_GETFD)
  let statusFlags = fcntl(fd, F_GETFL)
  guard fstat(fd, &descriptor) == 0,
    lstat(rootTrustedGitPath, &path) == 0,
    (UInt32(descriptor.st_mode) & UInt32(S_IFMT)) == UInt32(S_IFREG),
    descriptor.st_uid == 0,
    descriptor.st_dev == path.st_dev,
    descriptor.st_ino == path.st_ino,
    descriptor.st_mode == path.st_mode,
    (descriptor.st_mode & (S_IXUSR | S_IXGRP | S_IXOTH)) != 0,
    (descriptor.st_mode & (S_ISUID | S_ISGID)) == 0,
    (descriptor.st_mode & (S_IWGRP | S_IWOTH)) == 0,
    flags >= 0, (flags & FD_CLOEXEC) != 0,
    statusFlags >= 0, (statusFlags & O_ACCMODE) == O_RDONLY,
    validateRootTrustedFilesystemObject(rootTrustedGitPath),
    validateRootOwnedDirectory("/"),
    validateRootOwnedDirectory("/usr"),
    validateRootOwnedDirectory("/usr/bin")
  else { return false }
  return true
}

func validateTransferredGitReservation(_ fd: Int32) -> Bool {
  guard fd >= 0, fcntl(fd, F_SETFD, FD_CLOEXEC) == 0,
    validateGitReservationFD(fd)
  else { return false }
  let probe = open(rootTrustedGitPath, O_RDONLY | O_NOFOLLOW | O_CLOEXEC)
  guard probe >= 0 else { return false }
  defer { _ = close(probe) }
  errno = 0
  let result = flock(probe, LOCK_EX | LOCK_NB)
  if result == 0 {
    _ = flock(probe, LOCK_UN)
    return false
  }
  guard errno == EWOULDBLOCK || errno == EAGAIN else { return false }
  return flock(fd, LOCK_EX | LOCK_NB) == 0 && validateGitReservationFD(fd)
}

private func validateRootOwnedDirectory(_ path: String) -> Bool {
  var value = stat()
  return lstat(path, &value) == 0
    && (UInt32(value.st_mode) & UInt32(S_IFMT)) == UInt32(S_IFDIR)
    && value.st_uid == 0
    && (value.st_mode & (S_IWGRP | S_IWOTH)) == 0
    && validateRootTrustedFilesystemObject(path)
}

func killProcessGroup(_ pid: Int32) {
  guard pid > 1 else { return }
  if getpgid(pid) == pid {
    _ = kill(-pid, SIGKILL)
  }
  _ = kill(pid, SIGKILL)
}

func errnoDiagnostic() -> String {
  String(cString: strerror(errno))
}

struct SigningIdentity {
  let identifier: String
  let teamIdentifier: String
}


struct PreparedResponse: Decodable {
  let ok: Bool
  let spec: GitProcessSpec?
  let error: String?
}

struct GitProcessSpec: Decodable {
  let args: [String]
  let environment: [GitEnvironmentEntry]
}

struct GitEnvironmentEntry: Decodable {
  let key: String
  let value: String
}

struct DescriptorStat {
  let device: UInt64
  let inode: UInt64
  let mode: UInt32
  let size: UInt64
}


enum SpawnResult {
  case success(Int32)
  case failure(String)
}

func spawnSuspendedGit(
  spec: GitProcessSpec,
  cwdFD: Int32,
  stdinFD: Int32,
  stdoutFD: Int32,
  stderrFD: Int32,
  pidSlot: UnsafeMutablePointer<Int32>
) -> SpawnResult {
  guard spec.args.count <= 256, spec.environment.count <= 128 else {
    return .failure("typed Git process specification exceeded its count limit")
  }
  let arguments = ["/usr/bin/git"] + spec.args
  let environment = spec.environment.map { "\($0.key)=\($0.value)" }
  guard arguments.allSatisfy(validCString),
    spec.environment.allSatisfy({ validEnvironmentKey($0.key) && validCString($0.value) })
  else {
    return .failure("typed Git process specification contained an invalid string")
  }

  var actions: posix_spawn_file_actions_t?
  var attributes: posix_spawnattr_t?
  var error = posix_spawn_file_actions_init(&actions)
  guard error == 0 else { return .failure("posix_spawn file actions init failed: \(error)") }
  defer { posix_spawn_file_actions_destroy(&actions) }
  error = posix_spawnattr_init(&attributes)
  guard error == 0 else { return .failure("posix_spawn attributes init failed: \(error)") }
  defer { posix_spawnattr_destroy(&attributes) }

  for action in [
    posix_spawn_file_actions_addfchdir_np(&actions, cwdFD),
    posix_spawn_file_actions_adddup2(&actions, stdinFD, STDIN_FILENO),
    posix_spawn_file_actions_adddup2(&actions, stdoutFD, STDOUT_FILENO),
    posix_spawn_file_actions_adddup2(&actions, stderrFD, STDERR_FILENO),
  ] where action != 0 {
    return .failure("typed Git spawn file action failed: \(action)")
  }
  error = posix_spawnattr_setpgroup(&attributes, 0)
  guard error == 0 else { return .failure("typed Git process group setup failed: \(error)") }
  var defaultSignals = sigset_t()
  var signalMask = sigset_t()
  guard sigemptyset(&defaultSignals) == 0,
    sigaddset(&defaultSignals, SIGHUP) == 0,
    sigemptyset(&signalMask) == 0,
    posix_spawnattr_setsigdefault(&attributes, &defaultSignals) == 0,
    posix_spawnattr_setsigmask(&attributes, &signalMask) == 0
  else {
    return .failure("typed Git signal fail-safe setup failed")
  }
  let flags = Int16(
    POSIX_SPAWN_SETPGROUP | POSIX_SPAWN_SETSIGDEF | POSIX_SPAWN_SETSIGMASK
      | POSIX_SPAWN_START_SUSPENDED | POSIX_SPAWN_CLOEXEC_DEFAULT)
  error = posix_spawnattr_setflags(&attributes, flags)
  guard error == 0 else { return .failure("typed Git spawn flags failed: \(error)") }

  let argumentPointers = arguments.map { strdup($0) }
  let environmentPointers = environment.map { strdup($0) }
  guard argumentPointers.allSatisfy({ $0 != nil }),
    environmentPointers.allSatisfy({ $0 != nil })
  else {
    argumentPointers.forEach { free($0) }
    environmentPointers.forEach { free($0) }
    return .failure("typed Git spawn string allocation failed")
  }
  defer {
    argumentPointers.forEach { free($0) }
    environmentPointers.forEach { free($0) }
  }
  var argv = argumentPointers
  argv.append(nil)
  var envp = environmentPointers
  envp.append(nil)
  error = "/usr/bin/git".withCString { executable in
    argv.withUnsafeMutableBufferPointer { argvBuffer in
      envp.withUnsafeMutableBufferPointer { envBuffer in
        posix_spawn(
          pidSlot,
          executable,
          &actions,
          &attributes,
          argvBuffer.baseAddress,
          envBuffer.baseAddress
        )
      }
    }
  }
  guard error == 0, pidSlot.pointee > 0 else {
    return .failure("typed Git posix_spawn failed: \(error)")
  }
  return .success(pidSlot.pointee)
}



private func validCString(_ value: String) -> Bool {
  !value.utf8.contains(0) && value.utf8.count <= 128 * 1024
}

private func validEnvironmentKey(_ value: String) -> Bool {
  !value.isEmpty && validCString(value) && !value.contains("=")
}

func descriptorStat(_ fd: Int32) -> DescriptorStat? {
  var value = stat()
  guard fstat(fd, &value) == 0 else { return nil }
  return DescriptorStat(
    device: UInt64(bitPattern: Int64(value.st_dev)),
    inode: UInt64(value.st_ino),
    mode: UInt32(value.st_mode),
    size: UInt64(max(0, value.st_size))
  )
}



func decodePrepareError(_ encoded: String) -> String {
  guard let data = encoded.data(using: .utf8),
    let response = try? JSONDecoder().decode(PreparedResponse.self, from: data),
    let error = response.error,
    !error.isEmpty
  else { return "signed helper rejected the typed Git request" }
  return error
}

private func consumeChildWaitStatus(
  pid: Int32,
  options: Int32,
  expectedCode: Int32
) -> String? {
  while true {
    var consumed = siginfo_t()
    errno = 0
    if waitid(P_PID, id_t(pid), &consumed, options | WNOHANG) == 0 {
      if consumed.si_pid == 0 { return nil }
      guard consumed.si_pid == pid, consumed.si_code == expectedCode else {
        return "waitid consumed an unexpected typed Git child status"
      }
      return nil
    }
    if errno == EINTR { continue }
    return "waitid failed while consuming a nonterminal typed Git status: \(errnoDiagnostic())"
  }
}

func consumeInitialSuspendedChildStatus(pid: Int32) -> String? {
  while true {
    var stopped = siginfo_t()
    errno = 0
    if waitid(P_PID, id_t(pid), &stopped, WSTOPPED) == 0 {
      guard stopped.si_pid == pid, stopped.si_code == CLD_STOPPED else {
        return "typed Git did not enter its required suspended launch state"
      }
      return nil
    }
    if errno == EINTR { continue }
    return "waitid failed while confirming suspended typed Git: \(errnoDiagnostic())"
  }
}

func waitForTerminalChildStatus(pid: Int32) -> String? {
  while true {
    var observed = siginfo_t()
    errno = 0
    if waitid(P_PID, id_t(pid), &observed, WEXITED | WNOWAIT) != 0 {
      if errno == EINTR { continue }
      return "waitid failed for typed Git: \(errnoDiagnostic())"
    }
    guard observed.si_pid == pid else {
      return "waitid changed typed Git child identity"
    }
    switch observed.si_code {
    case CLD_EXITED, CLD_KILLED, CLD_DUMPED:
      return nil
    case CLD_STOPPED, CLD_TRAPPED:
      if let error = consumeChildWaitStatus(
        pid: pid,
        options: WSTOPPED,
        expectedCode: observed.si_code
      ) { return error }
    case CLD_CONTINUED:
      if let error = consumeChildWaitStatus(
        pid: pid,
        options: WCONTINUED,
        expectedCode: observed.si_code
      ) { return error }
    default:
      return "waitid returned unexpected typed Git status \(observed.si_code)"
    }
  }
}


func waitForProcessGroupExit(_ pid: Int32) -> Bool {
  guard pid > 0 else { return true }
  for _ in 0..<100 {
    errno = 0
    if kill(-pid, 0) != 0, errno == ESRCH {
      return true
    }
    usleep(10_000)
  }
  return false
}

func configureServiceChildReaping() -> Bool {
  let servicePID = getpid()
  if getpgid(0) != servicePID, setpgid(0, 0) != 0 { return false }
  guard servicePID > 1, getpgid(0) == servicePID else { return false }
  var desired = sigaction()
  desired.__sigaction_u.__sa_handler = SIG_DFL
  guard sigemptyset(&desired.sa_mask) == 0 else { return false }
  desired.sa_flags = 0
  guard sigaction(SIGCHLD, &desired, nil) == 0 else { return false }
  var actual = sigaction()
  guard sigaction(SIGCHLD, nil, &actual) == 0 else { return false }
  let actualHandler = unsafeBitCast(actual.__sigaction_u.__sa_handler, to: UInt.self)
  let defaultHandler = unsafeBitCast(SIG_DFL, to: UInt.self)
  return actualHandler == defaultHandler && (actual.sa_flags & SA_NOCLDWAIT) == 0
}

func appendDiagnostic(_ current: String?, _ addition: String) -> String {
  guard let current, !current.isEmpty else { return addition }
  return "\(current); \(addition)"
}

func currentSigningIdentity() -> SigningIdentity? {
  var code: SecCode?
  guard SecCodeCopySelf(SecCSFlags(), &code) == errSecSuccess, let code else { return nil }
  var staticCode: SecStaticCode?
  guard SecCodeCopyStaticCode(code, SecCSFlags(), &staticCode) == errSecSuccess,
    let staticCode
  else { return nil }
  var information: CFDictionary?
  guard SecCodeCopySigningInformation(
    staticCode,
    SecCSFlags(rawValue: kSecCSSigningInformation),
    &information
  ) == errSecSuccess,
    let dictionary = information as? [String: Any],
    let identifier = dictionary[kSecCodeInfoIdentifier as String] as? String,
    let teamIdentifier = dictionary[kSecCodeInfoTeamIdentifier as String] as? String,
    validTeamIdentifier(teamIdentifier)
  else { return nil }
  let identity = SigningIdentity(identifier: identifier, teamIdentifier: teamIdentifier)
  guard let requirement = developerIDRequirement(identity: identity),
    SecCodeCheckValidity(
      code,
      SecCSFlags(rawValue: kSecCSStrictValidate),
      requirement
    ) == errSecSuccess
  else { return nil }
  return identity
}

@available(macOS 12.0, *)
func installPeerRequirement(
  on connection: xpc_connection_t,
  expectedIdentifier: String,
  teamIdentifier: String
) -> String? {
  guard validSigningIdentifier(expectedIdentifier), validTeamIdentifier(teamIdentifier) else {
    return "invalid signing identity requirement"
  }
  let identity = SigningIdentity(
    identifier: expectedIdentifier,
    teamIdentifier: teamIdentifier
  )
  guard let requirement = developerIDRequirementString(identity: identity) else {
    return "invalid Developer ID signing requirement"
  }
  let result = requirement.withCString {
    xpc_connection_set_peer_code_signing_requirement(connection, $0)
  }
  return result == 0 ? nil : "peer signing requirement failed with error \(result)"
}

private func validateStaticDeveloperIDCode(
  at url: URL,
  identifier: String,
  teamIdentifier: String
) -> Bool {
  let identity = SigningIdentity(identifier: identifier, teamIdentifier: teamIdentifier)
  guard let requirement = developerIDRequirement(identity: identity) else { return false }
  var code: SecStaticCode?
  guard SecStaticCodeCreateWithPath(url as CFURL, SecCSFlags(), &code) == errSecSuccess,
    let code
  else { return false }
  let flags = SecCSFlags(rawValue: kSecCSStrictValidate | kSecCSCheckAllArchitectures)
  return SecStaticCodeCheckValidity(code, flags, requirement) == errSecSuccess
}

private func developerIDRequirement(identity: SigningIdentity) -> SecRequirement? {
  guard let source = developerIDRequirementString(identity: identity) else { return nil }
  var requirement: SecRequirement?
  guard SecRequirementCreateWithString(
    source as CFString,
    SecCSFlags(),
    &requirement
  ) == errSecSuccess else { return nil }
  return requirement
}

private func developerIDRequirementString(identity: SigningIdentity) -> String? {
  guard validSigningIdentifier(identity.identifier),
    validTeamIdentifier(identity.teamIdentifier)
  else { return nil }
  return "anchor apple generic and identifier \"\(identity.identifier)\""
    + " and certificate 1[field.1.2.840.113635.100.6.2.6] exists"
    + " and certificate leaf[field.1.2.840.113635.100.6.1.13] exists"
    + " and certificate leaf[subject.OU] = \"\(identity.teamIdentifier)\""
}

private func validSigningIdentifier(_ value: String) -> Bool {
  !value.isEmpty
    && value.utf8.allSatisfy {
      ($0 >= 48 && $0 <= 57) || ($0 >= 65 && $0 <= 90) || ($0 >= 97 && $0 <= 122)
        || $0 == 45 || $0 == 46
    }
}

func validTeamIdentifier(_ value: String) -> Bool {
  !value.isEmpty
    && value.utf8.allSatisfy { ($0 >= 48 && $0 <= 57) || ($0 >= 65 && $0 <= 90) }
}
