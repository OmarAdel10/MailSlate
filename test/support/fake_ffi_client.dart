import 'dart:async';

import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/core/failures/failures.dart';

/// Test double for [FfiClient]: never loads Rust, never touches a daemon.
class FakeFfiClient extends FfiClient {
  FakeFfiClient({
    this.status = const DaemonStatus(state: DaemonState.unavailable),
    this.pingResult = const PingInfo(
      protocolVersion: 1,
      daemonVersion: '0.0.0-test',
    ),
    this.startFailure,
    this.statusFailure,
    this.statusThrow,
    this.logDir = '',
  });

  /// Returned by [daemonStatus] when [statusFailure] is null.
  DaemonStatus status;

  /// Returned by [startDaemon] and [ping] when no failure is set.
  PingInfo pingResult;

  /// When set, [startDaemon] throws it.
  AppFailure? startFailure;

  /// When set, [daemonStatus] throws it.
  AppFailure? statusFailure;

  /// When set, [daemonStatus] throws it (any object, for generic catches).
  Object? statusThrow;

  /// Returned by [daemonLogDir].
  String logDir;

  /// How often [startDaemon] was called.
  int startCalls = 0;

  /// How often [daemonStatus] was called.
  int statusCalls = 0;

  /// When set, [daemonStatus] parks until the gate completes.
  Completer<void>? statusGate;

  @override
  Future<void> initApp() async {}

  @override
  Future<DaemonStatus> daemonStatus() async {
    statusCalls += 1;
    final gate = statusGate;
    if (gate != null) {
      await gate.future;
    }
    final thrown = statusThrow;
    if (thrown != null) {
      throw thrown;
    }
    final failure = statusFailure;
    if (failure != null) {
      throw failure;
    }
    return status;
  }

  @override
  Future<PingInfo> startDaemon() async {
    startCalls += 1;
    final failure = startFailure;
    if (failure != null) {
      throw failure;
    }
    return pingResult;
  }

  @override
  Future<String> daemonLogDir() async => logDir;

  @override
  Future<PingInfo> ping() async => pingResult;
}
