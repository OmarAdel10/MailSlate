import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/core/failures/failures.dart';
import 'package:mailslate/src/rust/api/mailbox.dart';
import 'package:mailslate/src/rust/api/reader.dart';
import 'package:mailslate/src/rust/api/system.dart';
import 'package:mailslate/src/rust/api/threading.dart';
import 'package:mailslate/src/rust/frb_generated.dart';

/// Fake of the generated [RustLibApi]: answers without the native library.
///
/// [RustLib.initMock] may run only once per process, so a single instance is
/// created in `setUpAll` and reconfigured per test via [reset].
class FakeRustLibApi extends RustLibApi {
  DaemonStatus? status;
  PingInfo? pingResult;
  DaemonError? error;

  /// Error thrown by every accounts call when set.
  DaemonError? accountsError;

  /// Canned [FfiClient.listAccounts] result.
  List<AccountSummary>? accountsList;

  /// Captured add-imap arguments.
  ImapDraft? lastAddDraft;
  String? lastAddPassword;

  /// Captured start-oauth provider.
  String? lastOauthProvider;

  /// Captured reconnect account id.
  int? lastReauthId;

  /// Captured complete-oauth arguments.
  String? lastCompleteDisplayName;
  String? lastCompleteEmail;
  String? lastCompleteColor;

  /// Captured update/remove arguments.
  int? lastUpdateId;
  AccountPatch? lastPatch;
  int? lastRemoveId;

  /// Captured test-connection arguments.
  ImapDraft? lastTestDraft;
  String? lastTestPassword;

  /// Whether abort-oauth-account was called.
  bool abortCalled = false;

  int pingCalls = 0;

  /// Stream the fake [FfiClient.events] can subscribe to in tests.
  Stream<AppEvent> eventStream = const Stream<AppEvent>.empty();

  void reset() {
    status = null;
    pingResult = null;
    error = null;
    accountsError = null;
    accountsList = null;
    lastAddDraft = null;
    lastAddPassword = null;
    lastOauthProvider = null;
    lastReauthId = null;
    lastCompleteDisplayName = null;
    lastCompleteEmail = null;
    lastCompleteColor = null;
    lastUpdateId = null;
    lastPatch = null;
    lastRemoveId = null;
    lastTestDraft = null;
    lastTestPassword = null;
    abortCalled = false;
    pingCalls = 0;
    eventStream = const Stream<AppEvent>.empty();
  }

  @override
  Stream<AppEvent> crateApiSystemAppEvents() => eventStream;

  @override
  Future<List<AccountSummary>> crateApiAccountsListAccounts() async {
    final failure = accountsError;
    if (failure != null) {
      throw failure;
    }
    return accountsList ?? const [];
  }

  @override
  Future<int> crateApiAccountsAddImapAccount({
    required ImapDraft draft,
    required String password,
  }) async {
    final failure = accountsError;
    if (failure != null) {
      throw failure;
    }
    lastAddDraft = draft;
    lastAddPassword = password;
    return 7;
  }

  @override
  Future<String> crateApiAccountsStartOauth({required String provider}) async {
    final failure = accountsError;
    if (failure != null) {
      throw failure;
    }
    lastOauthProvider = provider;
    return 'https://accounts.example.test/auth';
  }

  @override
  Future<String> crateApiAccountsStartReauth({required int accountId}) async {
    final failure = accountsError;
    if (failure != null) {
      throw failure;
    }
    lastReauthId = accountId;
    return 'https://accounts.example.test/reauth';
  }

  @override
  Future<int> crateApiAccountsCompleteOauthAccount({
    required String displayName,
    required String email,
    String? color,
  }) async {
    final failure = accountsError;
    if (failure != null) {
      throw failure;
    }
    lastCompleteDisplayName = displayName;
    lastCompleteEmail = email;
    lastCompleteColor = color;
    return 11;
  }

  @override
  Future<void> crateApiAccountsUpdateAccount({
    required int accountId,
    required AccountPatch patch,
  }) async {
    final failure = accountsError;
    if (failure != null) {
      throw failure;
    }
    lastUpdateId = accountId;
    lastPatch = patch;
  }

  @override
  Future<void> crateApiAccountsRemoveAccount({required int accountId}) async {
    final failure = accountsError;
    if (failure != null) {
      throw failure;
    }
    lastRemoveId = accountId;
  }

  @override
  Future<ConnectionTest> crateApiAccountsTestConnection({
    required ImapDraft draft,
    required String password,
  }) async {
    final failure = accountsError;
    if (failure != null) {
      throw failure;
    }
    lastTestDraft = draft;
    lastTestPassword = password;
    return const ConnectionTest(
      imap: ConnectionStep(ok: true),
      smtp: ConnectionStep(ok: true),
    );
  }

  @override
  Future<void> crateApiAccountsAbortOauthAccount() async {
    final failure = accountsError;
    if (failure != null) {
      throw failure;
    }
    abortCalled = true;
  }

  @override
  Future<SearchResult> crateApiSearchSearch({
    required int accountId,
    required String query,
  }) async {
    final failure = accountsError;
    if (failure != null) {
      throw failure;
    }
    return const SearchResult(messages: [], total: 0);
  }

  @override
  Future<ThreadList> crateApiThreadingListThreads({
    required PlatformInt64 accountId,
    required int limit,
    PlatformInt64? lastDate,
    PlatformInt64? lastId,
  }) async {
    final failure = accountsError;
    if (failure != null) {
      throw failure;
    }
    return const ThreadList(threads: [], total: 0);
  }

  @override
  Future<ThreadMessageList> crateApiThreadingListThreadMessages({
    required PlatformInt64 threadId,
    required int limit,
    PlatformInt64? lastDate,
    PlatformInt64? lastId,
  }) async {
    final failure = accountsError;
    if (failure != null) {
      throw failure;
    }
    return const ThreadMessageList(messages: [], total: 0);
  }

  @override
  Future<MessageResult> crateApiReaderGetMessage({
    required PlatformInt64 accountId,
    required PlatformInt64 messageId,
  }) async {
    final failure = accountsError;
    if (failure != null) {
      throw failure;
    }
    return const MessageResult(
      message: MessageDetail(
        id: 1,
        accountId: 42,
        subject: 'Reader subject',
        fromJson: '[{"name":"Alice","address":"alice@example.com"}]',
        toJson: '[{"name":"Bob","address":"bob@example.com"}]',
        dateUtc: 1000,
        snippet: 'reader snippet',
        hasAttachments: false,
        bodyState: 'full',
      ),
    );
  }

  @override
  Future<MessageBody> crateApiReaderGetMessageBody({
    required PlatformInt64 messageId,
  }) async {
    final failure = accountsError;
    if (failure != null) {
      throw failure;
    }
    return const MessageBody(
      messageId: 1,
      textBody: 'Plain text body',
      htmlClean: '<b>HTML body</b>',
    );
  }

  @override
  Future<AttachmentList> crateApiReaderGetAttachments({
    required PlatformInt64 messageId,
  }) async {
    final failure = accountsError;
    if (failure != null) {
      throw failure;
    }
    return const AttachmentList(attachments: [], total: 0);
  }

  @override
  Future<void> crateApiReaderMarkRead({
    required PlatformInt64 accountId,
    required PlatformInt64 messageId,
    required bool seen,
  }) async {
    final failure = accountsError;
    if (failure != null) {
      throw failure;
    }
  }

  @override
  Future<MailboxList> crateApiMailboxListMailboxes({
    required PlatformInt64 accountId,
    required int limit,
    PlatformInt64? lastSortOrder,
    String? lastKind,
    String? lastName,
  }) async {
    final failure = accountsError;
    if (failure != null) {
      throw failure;
    }
    return const MailboxList(mailboxes: [], total: 0);
  }

  @override
  Future<DaemonStatus> crateApiSystemDaemonStatus() async {
    final value = status;
    if (value == null) {
      throw StateError('no status configured');
    }
    return value;
  }

  @override
  Future<void> crateApiSystemInitApp() async {}

  @override
  Future<String> crateApiSystemDaemonLogDir() async => '';

  @override
  Future<PingInfo> crateApiSystemPing() async {
    pingCalls += 1;
    final failure = error;
    if (failure != null) {
      throw failure;
    }
    return pingResult ??
        const PingInfo(protocolVersion: 1, daemonVersion: '0.0.0-test');
  }

  @override
  Future<PingInfo> crateApiSystemStartDaemon() async {
    return pingResult ??
        const PingInfo(protocolVersion: 1, daemonVersion: '0.0.0-test');
  }
}

void main() {
  final api = FakeRustLibApi();

  setUpAll(() {
    RustLib.initMock(api: api);
  });

  setUp(api.reset);

  test('ping surfaces a DaemonError as an AppFailure via the guard', () async {
    api.error = DaemonError(
      kind: 'daemon_unavailable',
      message: 'could not connect to /tmp/daemon.sock',
      retryable: true,
    );

    await expectLater(
      const FfiClient().ping(),
      throwsA(
        isA<DaemonUnavailableFailure>()
            .having(
              (failure) => failure.message,
              'message',
              'could not connect to /tmp/daemon.sock',
            )
            .having((failure) => failure.retryable, 'retryable', isTrue),
      ),
    );
    expect(api.pingCalls, 1);
  });

  test('daemonStatus and startDaemon pass values through', () async {
    api.status = const DaemonStatus(
      state: DaemonState.running,
      daemonVersion: '9.9.9',
      protocolVersion: 1,
    );
    api.pingResult = const PingInfo(protocolVersion: 1, daemonVersion: '9.9.9');

    const client = FfiClient();
    final status = await client.daemonStatus();
    expect(status.state, DaemonState.running);
    expect(status.daemonVersion, '9.9.9');
    expect(status.protocolVersion, 1);

    final info = await client.startDaemon();
    expect(info.daemonVersion, '9.9.9');
    expect(info.protocolVersion, 1);
  });

  test('events forwards the FRB AppEvent stream', () async {
    api.eventStream = Stream<AppEvent>.fromIterable([
      const AppEvent.accountStatus(account: 1, state: 'syncing'),
      const AppEvent.other(name: 'future_thing'),
    ]);

    final events = await const FfiClient().events().toList();

    expect(events, [
      const AppEvent.accountStatus(account: 1, state: 'syncing'),
      const AppEvent.other(name: 'future_thing'),
    ]);
  });

  test('listAccounts passes the summary rows through', () async {
    api.accountsList = const [
      AccountSummary(
        id: 1,
        displayName: 'Work',
        email: 'work@example.test',
        provider: 'google',
        authType: 'oauth2',
        color: 'blue',
        syncWindowDays: 30,
      ),
      AccountSummary(
        id: 2,
        displayName: 'Private',
        email: 'private@example.test',
        provider: 'imap',
        authType: 'password',
        color: null,
        syncWindowDays: 90,
      ),
    ];

    final accounts = await const FfiClient().listAccounts();

    expect(accounts, api.accountsList);
  });

  test('addImapAccount forwards the draft and password', () async {
    const draft = ImapDraft(
      displayName: 'Private',
      email: 'private@example.test',
      imapHost: 'imap.example.test',
      imapPort: 993,
      imapSecurity: 'ssl',
      smtpHost: 'smtp.example.test',
      smtpPort: 465,
      smtpSecurity: 'ssl',
    );

    final id = await const FfiClient().addImapAccount(
      draft: draft,
      password: 'secret',
    );

    expect(id, 7);
    expect(api.lastAddDraft, draft);
    expect(api.lastAddPassword, 'secret');
  });

  test('addImapAccount maps DaemonError to an AppFailure', () async {
    api.accountsError = DaemonError(
      kind: 'validation',
      message: 'bad account',
      retryable: false,
    );

    await expectLater(
      const FfiClient().addImapAccount(
        draft: ImapDraft(
          email: 'private@example.test',
          imapHost: 'imap.example.test',
          imapPort: 993,
          imapSecurity: 'ssl',
          smtpHost: 'smtp.example.test',
          smtpPort: 465,
          smtpSecurity: 'ssl',
        ),
        password: 'secret',
      ),
      throwsA(
        isA<ValidationFailure>().having(
          (failure) => failure.message,
          'message',
          'bad account',
        ),
      ),
    );
  });

  test('startOauth returns the authorize url for the provider', () async {
    final url = await const FfiClient().startOauth(provider: 'google');

    expect(url, 'https://accounts.example.test/auth');
    expect(api.lastOauthProvider, 'google');
  });

  test('startReauth forwards the account id and returns the url', () async {
    final url = await const FfiClient().startReauth(accountId: 5);

    expect(url, 'https://accounts.example.test/reauth');
    expect(api.lastReauthId, 5);
  });

  test('completeOauthAccount forwards the fields and returns the id', () async {
    final id = await const FfiClient().completeOauthAccount(
      displayName: 'Work',
      email: 'work@example.test',
      color: 'blue',
    );

    expect(id, 11);
    expect(api.lastCompleteDisplayName, 'Work');
    expect(api.lastCompleteEmail, 'work@example.test');
    expect(api.lastCompleteColor, 'blue');
  });

  test(
    'updateAccount and removeAccount pass their arguments through',
    () async {
      const patch = AccountPatch(
        displayName: 'Renamed',
        clearColor: true,
        syncWindowDays: 30,
      );

      await const FfiClient().updateAccount(accountId: 3, patch: patch);
      await const FfiClient().removeAccount(accountId: 9);

      expect(api.lastUpdateId, 3);
      expect(api.lastPatch, patch);
      expect(api.lastRemoveId, 9);
    },
  );

  test('updateAccount maps DaemonError to an AppFailure', () async {
    api.accountsError = DaemonError(
      kind: 'validation',
      message: 'bad patch',
      retryable: false,
    );

    await expectLater(
      const FfiClient().updateAccount(
        accountId: 3,
        patch: const AccountPatch(clearColor: true),
      ),
      throwsA(
        isA<ValidationFailure>().having(
          (failure) => failure.message,
          'message',
          'bad patch',
        ),
      ),
    );
  });

  test('removeAccount maps DaemonError to an AppFailure', () async {
    api.accountsError = DaemonError(
      kind: 'validation',
      message: 'bad account',
      retryable: false,
    );

    await expectLater(
      const FfiClient().removeAccount(accountId: 9),
      throwsA(
        isA<ValidationFailure>().having(
          (failure) => failure.message,
          'message',
          'bad account',
        ),
      ),
    );
  });

  test('testConnection forwards the draft and password', () async {
    const draft = ImapDraft(
      email: 'private@example.test',
      imapHost: 'imap.example.test',
      imapPort: 993,
      imapSecurity: 'ssl',
      smtpHost: 'smtp.example.test',
      smtpPort: 465,
      smtpSecurity: 'ssl',
    );

    final test = await const FfiClient().testConnection(
      draft: draft,
      password: 'secret',
    );

    expect(test.imap.ok, isTrue);
    expect(test.smtp.ok, isTrue);
    expect(api.lastTestDraft, draft);
    expect(api.lastTestPassword, 'secret');
  });

  test('testConnection maps DaemonError to an AppFailure', () async {
    api.accountsError = DaemonError(
      kind: 'validation',
      message: 'bad account',
      retryable: false,
    );

    await expectLater(
      const FfiClient().testConnection(
        draft: ImapDraft(
          email: 'private@example.test',
          imapHost: 'imap.example.test',
          imapPort: 993,
          imapSecurity: 'ssl',
          smtpHost: 'smtp.example.test',
          smtpPort: 465,
          smtpSecurity: 'ssl',
        ),
        password: 'secret',
      ),
      throwsA(
        isA<ValidationFailure>().having(
          (failure) => failure.message,
          'message',
          'bad account',
        ),
      ),
    );
  });

  test('abortOauthAccount releases the session and maps errors', () async {
    await const FfiClient().abortOauthAccount();
    expect(api.abortCalled, isTrue);

    api.accountsError = DaemonError(
      kind: 'validation',
      message: 'session gone',
      retryable: false,
    );
    await expectLater(
      const FfiClient().abortOauthAccount(),
      throwsA(
        isA<ValidationFailure>().having(
          (failure) => failure.message,
          'message',
          'session gone',
        ),
      ),
    );
  });
}
