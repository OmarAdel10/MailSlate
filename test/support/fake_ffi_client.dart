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
    this.eventStream = _emptyEvents,
    this.accounts = const [],
    this.accountsFailure,
    this.addImapResult = 7,
    this.addImapFailure,
    this.oauthUrl = 'https://accounts.example.test/auth',
    this.startOauthFailure,
    this.completeOauthResult = 11,
    this.completeOauthFailure,
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

  /// Builds the stream returned by [FfiClient.events]; fresh per call,
  /// like the real bridge (each `appEvents()` call dials a new Rust sink).
  Stream<AppEvent> Function() eventStream;

  static Stream<AppEvent> _emptyEvents() => const Stream<AppEvent>.empty();

  /// Canned [FfiClient.listAccounts] result.
  List<AccountSummary> accounts;

  /// When set, every accounts call throws it.
  AppFailure? accountsFailure;

  /// Returned by [FfiClient.addImapAccount] when [addImapFailure] is null.
  int addImapResult;

  /// When set, [FfiClient.addImapAccount] throws it.
  AppFailure? addImapFailure;

  /// Returned by [FfiClient.startOauth] when [startOauthFailure] is null.
  String oauthUrl;

  /// When set, [FfiClient.startOauth] throws it.
  AppFailure? startOauthFailure;

  /// Returned by [FfiClient.completeOauthAccount] when null failure.
  int completeOauthResult;

  /// When set, [FfiClient.completeOauthAccount] throws it.
  AppFailure? completeOauthFailure;

  /// How often [FfiClient.listAccounts] was called.
  int listAccountsCalls = 0;

  /// How often [FfiClient.addImapAccount] was called.
  int addImapCalls = 0;

  /// How often [FfiClient.startOauth] was called.
  int startOauthCalls = 0;

  /// How often [FfiClient.completeOauthAccount] was called.
  int completeOauthCalls = 0;

  /// How often [FfiClient.updateAccount] was called.
  int updateCalls = 0;

  /// How often [FfiClient.removeAccount] was called.
  int removeCalls = 0;

  /// Captured [FfiClient.addImapAccount] arguments.
  ImapDraft? lastAddDraft;
  String? lastAddPassword;

  /// Captured [FfiClient.startOauth] provider.
  String? lastOauthProvider;

  /// Captured [FfiClient.completeOauthAccount] arguments.
  String? lastCompleteDisplayName;
  String? lastCompleteEmail;
  String? lastCompleteColor;

  /// Captured [FfiClient.updateAccount] arguments.
  int? lastUpdateId;
  AccountPatch? lastPatch;

  /// Captured [FfiClient.removeAccount] argument.
  int? lastRemoveId;

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
  Stream<AppEvent> events() => eventStream();

  @override
  Future<PingInfo> ping() async => pingResult;

  @override
  Future<List<AccountSummary>> listAccounts() async {
    listAccountsCalls += 1;
    final failure = accountsFailure;
    if (failure != null) {
      throw failure;
    }
    return accounts;
  }

  @override
  Future<int> addImapAccount({
    required ImapDraft draft,
    required String password,
  }) async {
    addImapCalls += 1;
    lastAddDraft = draft;
    lastAddPassword = password;
    final failure = addImapFailure;
    if (failure != null) {
      throw failure;
    }
    return addImapResult;
  }

  @override
  Future<String> startOauth({required String provider}) async {
    startOauthCalls += 1;
    lastOauthProvider = provider;
    final failure = startOauthFailure;
    if (failure != null) {
      throw failure;
    }
    return oauthUrl;
  }

  @override
  Future<int> completeOauthAccount({
    required String displayName,
    required String email,
    String? color,
  }) async {
    completeOauthCalls += 1;
    lastCompleteDisplayName = displayName;
    lastCompleteEmail = email;
    lastCompleteColor = color;
    final failure = completeOauthFailure;
    if (failure != null) {
      throw failure;
    }
    return completeOauthResult;
  }

  @override
  Future<void> updateAccount({
    required int accountId,
    required AccountPatch patch,
  }) async {
    updateCalls += 1;
    lastUpdateId = accountId;
    lastPatch = patch;
    final failure = accountsFailure;
    if (failure != null) {
      throw failure;
    }
  }

  @override
  Future<void> removeAccount({required int accountId}) async {
    removeCalls += 1;
    lastRemoveId = accountId;
    final failure = accountsFailure;
    if (failure != null) {
      throw failure;
    }
  }
}
