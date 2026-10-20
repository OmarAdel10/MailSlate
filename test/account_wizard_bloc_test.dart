import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:mailslate/core/failures/failures.dart';
import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/features/accounts/wizard/account_wizard_bloc.dart';

import 'support/fake_ffi_client.dart';
import 'support/fake_url_opener.dart';

const _imapBothOk = ConnectionTest(
  imap: ConnectionStep(ok: true),
  smtp: ConnectionStep(ok: true),
);

ImapDraft _draft() => ImapDraft(
  email: 'work@example.com',
  imapHost: 'imap.example.com',
  imapPort: 993,
  imapSecurity: 'tls',
  smtpHost: 'smtp.example.com',
  smtpPort: 465,
  smtpSecurity: 'tls',
);

void main() {
  late FakeFfiClient client;
  late FakeUrlOpener urlOpener;
  late StreamController<AppEvent> events;
  late AccountWizardBloc wizard;

  setUp(() {
    client = FakeFfiClient();
    urlOpener = FakeUrlOpener();
    events = StreamController<AppEvent>();
  });

  tearDown(() async {
    await wizard.close();
    await events.close();
  });

  AccountWizardBloc build({
    bool firstAccount = true,
    Duration oauthDeadline = const Duration(minutes: 2),
  }) {
    wizard = AccountWizardBloc(
      client,
      urlOpener,
      events: events.stream,
      firstAccount: firstAccount,
      oauthDeadline: oauthDeadline,
    );
    return wizard;
  }

  group('entry step', () {
    test('starts on the welcome step for the first account', () {
      wizard = build();
      expect(wizard.state, const WizardWelcome());
    });

    test('starts on the provider pick for additional accounts', () {
      wizard = build(firstAccount: false);
      expect(wizard.state, const ChooseProvider());
    });

    test('welcome proceeds to the provider pick', () async {
      wizard = build();
      wizard.add(const ConnectAccountRequested());
      await wizard.stream.firstWhere((state) => state is ChooseProvider);

      expect(wizard.state, const ChooseProvider());
    });
  });

  group('provider pick', () {
    test('picking gmail starts oauth and opens the browser', () async {
      wizard = build();
      wizard.add(const ProviderPicked(provider: WizardProvider.gmail));
      await wizard.stream.firstWhere((state) => state is WaitingForOauth);

      expect(client.lastOauthProvider, 'google');
      expect(urlOpener.openedUrls, [client.oauthUrl]);
      expect(
        wizard.state,
        WaitingForOauth(provider: WizardProvider.gmail, url: client.oauthUrl),
      );
    });

    test('picking outlook uses the microsoft provider', () async {
      wizard = build();
      wizard.add(const ProviderPicked(provider: WizardProvider.outlook));
      await wizard.stream.firstWhere((state) => state is WaitingForOauth);

      expect(client.lastOauthProvider, 'microsoft');
    });

    test('picking other imap shows the details form', () async {
      wizard = build();
      wizard.add(const ProviderPicked(provider: WizardProvider.otherImap));
      await wizard.stream.firstWhere((state) => state is EnterImapDetails);

      expect(wizard.state, const EnterImapDetails());
    });

    test('a failed start returns to the pick with the failure note', () async {
      client.startOauthFailure = const NetworkFailure(
        message: 'cannot reach the daemon',
      );
      wizard = build();
      wizard.add(const ProviderPicked(provider: WizardProvider.gmail));
      await wizard.stream.firstWhere((state) => state is ChooseProvider);

      expect((wizard.state as ChooseProvider).note, 'cannot reach the daemon');
    });
  });

  group('oauth waiting', () {
    test('oauth completion finishes the account', () async {
      wizard = build();
      wizard.add(const ProviderPicked(provider: WizardProvider.gmail));
      await wizard.stream.firstWhere((state) => state is WaitingForOauth);

      events.add(const AppEvent.oauthCompleted(email: 'user@gmail.com'));
      await wizard.stream.firstWhere((state) => state is AccountConnected);

      expect(client.completeOauthCalls, 1);
      expect(client.lastCompleteEmail, 'user@gmail.com');
      // USER_FLOW §3 has no name step: the address is the display name.
      expect(client.lastCompleteDisplayName, 'user@gmail.com');
      expect(wizard.state, const AccountConnected(accountId: 11));
    });

    test('denied consent returns to the pick with a note', () async {
      wizard = build();
      wizard.add(const ProviderPicked(provider: WizardProvider.gmail));
      await wizard.stream.firstWhere((state) => state is WaitingForOauth);

      events.add(const AppEvent.oauthFailed(reason: 'consent denied'));
      await wizard.stream.firstWhere((state) => state is ChooseProvider);

      expect((wizard.state as ChooseProvider).note, 'consent denied');
    });

    test('the browser can be reopened from the waiting step', () async {
      wizard = build();
      wizard.add(const ProviderPicked(provider: WizardProvider.gmail));
      await wizard.stream.firstWhere((state) => state is WaitingForOauth);

      wizard.add(const ReopenBrowserRequested());
      await Future<void>.delayed(Duration.zero);

      expect(urlOpener.openedUrls, [client.oauthUrl, client.oauthUrl]);
    });

    test('waiting times out after the deadline', () async {
      wizard = build(oauthDeadline: const Duration(milliseconds: 20));
      wizard.add(const ProviderPicked(provider: WizardProvider.gmail));
      await wizard.stream.firstWhere((state) => state is WaitingForOauth);

      await wizard.stream.firstWhere((state) => state is OauthTimedOut);

      expect(
        wizard.state,
        OauthTimedOut(provider: WizardProvider.gmail, url: client.oauthUrl),
      );
    });

    test('try again after a timeout restarts oauth', () async {
      wizard = build(oauthDeadline: const Duration(milliseconds: 20));
      wizard.add(const ProviderPicked(provider: WizardProvider.gmail));
      await wizard.stream.firstWhere((state) => state is WaitingForOauth);
      await wizard.stream.firstWhere((state) => state is OauthTimedOut);

      wizard.add(const OauthRetryRequested());
      await Future<void>.delayed(Duration.zero);

      expect(client.startOauthCalls, 2);
      expect(urlOpener.openedUrls.length, 2);
      expect(
        wizard.state,
        WaitingForOauth(provider: WizardProvider.gmail, url: client.oauthUrl),
      );
    });

    test('the timeout timer stops once the account is connected', () async {
      wizard = build(oauthDeadline: const Duration(milliseconds: 20));
      wizard.add(const ProviderPicked(provider: WizardProvider.gmail));
      await wizard.stream.firstWhere((state) => state is WaitingForOauth);

      events.add(const AppEvent.oauthCompleted(email: 'user@gmail.com'));
      await wizard.stream.firstWhere((state) => state is AccountConnected);
      await Future<void>.delayed(const Duration(milliseconds: 60));

      expect(wizard.state, const AccountConnected(accountId: 11));
    });

    test('a failed completion returns to the pick with a note', () async {
      client.completeOauthFailure = const ServerRejectedFailure(
        message: 'token exchange failed',
      );
      wizard = build();
      wizard.add(const ProviderPicked(provider: WizardProvider.gmail));
      await wizard.stream.firstWhere((state) => state is WaitingForOauth);

      events.add(const AppEvent.oauthCompleted(email: 'user@gmail.com'));
      await wizard.stream.firstWhere((state) => state is ChooseProvider);

      expect((wizard.state as ChooseProvider).note, 'token exchange failed');
    });

    test('oauth events are ignored outside the waiting step', () async {
      wizard = build();

      events.add(const AppEvent.oauthCompleted(email: 'user@gmail.com'));
      events.add(const AppEvent.oauthFailed(reason: 'too late'));
      await Future<void>.delayed(Duration.zero);

      expect(client.completeOauthCalls, 0);
      expect(wizard.state, const WizardWelcome());
    });
  });

  group('other imap details', () {
    test('the connection test stores the results', () async {
      wizard = build();
      wizard.add(const ProviderPicked(provider: WizardProvider.otherImap));
      await wizard.stream.firstWhere((state) => state is EnterImapDetails);

      wizard.add(
        ImapConnectionTestRequested(draft: _draft(), password: 'secret-pw'),
      );
      await wizard.stream.firstWhere(
        (state) => state is EnterImapDetails && state.lastTest != null,
      );

      expect(client.testConnectionCalls, 1);
      expect(client.lastTestDraft, _draft());
      expect(client.lastTestPassword, 'secret-pw');
      expect((wizard.state as EnterImapDetails).lastTest, _imapBothOk);
      expect((wizard.state as EnterImapDetails).note, isNull);
    });

    test('a failed test keeps the form with the failure note', () async {
      client.testConnectionFailure = const AuthFailedFailure(
        message: 'imap login rejected',
      );
      wizard = build();
      wizard.add(const ProviderPicked(provider: WizardProvider.otherImap));
      await wizard.stream.firstWhere((state) => state is EnterImapDetails);

      wizard.add(
        ImapConnectionTestRequested(draft: _draft(), password: 'secret-pw'),
      );
      await wizard.stream.firstWhere(
        (state) => state is EnterImapDetails && state.note != null,
      );

      expect((wizard.state as EnterImapDetails).note, 'imap login rejected');
      expect((wizard.state as EnterImapDetails).lastTest, isNull);
    });

    test('saving the imap account connects it', () async {
      wizard = build();
      wizard.add(const ProviderPicked(provider: WizardProvider.otherImap));
      await wizard.stream.firstWhere((state) => state is EnterImapDetails);

      wizard.add(
        ImapAccountSaveRequested(draft: _draft(), password: 'secret-pw'),
      );
      await wizard.stream.firstWhere((state) => state is AccountConnected);

      expect(client.addImapCalls, 1);
      expect(client.lastAddDraft, _draft());
      expect(wizard.state, const AccountConnected(accountId: 7));
    });

    test('a failed save keeps the form and the test results', () async {
      client.addImapFailure = const NetworkFailure(message: 'daemon went away');
      wizard = build();
      wizard.add(const ProviderPicked(provider: WizardProvider.otherImap));
      await wizard.stream.firstWhere((state) => state is EnterImapDetails);
      wizard.add(
        ImapConnectionTestRequested(draft: _draft(), password: 'secret-pw'),
      );
      await wizard.stream.firstWhere(
        (state) => state is EnterImapDetails && state.lastTest != null,
      );

      wizard.add(
        ImapAccountSaveRequested(draft: _draft(), password: 'secret-pw'),
      );
      await wizard.stream.firstWhere(
        (state) => state is EnterImapDetails && state.note != null,
      );

      expect((wizard.state as EnterImapDetails).note, 'daemon went away');
      expect((wizard.state as EnterImapDetails).lastTest, _imapBothOk);
    });

    test('back returns to the provider pick', () async {
      wizard = build();
      wizard.add(const ProviderPicked(provider: WizardProvider.otherImap));
      await wizard.stream.firstWhere((state) => state is EnterImapDetails);

      wizard.add(const BackToProviderPickRequested());
      await wizard.stream.firstWhere((state) => state is ChooseProvider);

      expect(wizard.state, const ChooseProvider());
    });
  });
}
