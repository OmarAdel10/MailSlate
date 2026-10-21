import 'package:flutter/material.dart';
import 'package:flutter_bloc/flutter_bloc.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mailslate/core/di/di.dart';
import 'package:mailslate/core/failures/failures.dart';
import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/features/accounts/account_gate.dart';
import 'package:mailslate/features/accounts/wizard/account_wizard_bloc.dart';
import 'package:mailslate/core/theme/theme.dart';
import 'package:mailslate/features/shell/app_events_bloc.dart';

import 'support/fake_ffi_client.dart';
import 'support/fake_url_opener.dart';

const _workAccount = AccountSummary(
  id: 1,
  displayName: 'Work',
  email: 'work@example.com',
  provider: 'imap',
  authType: 'password',
  syncWindowDays: 30,
);

void main() {
  setUp(getIt.reset);
  tearDown(getIt.reset);

  Future<void> pumpGate(WidgetTester tester) {
    // Mirrors the app root: the events bloc sits above the gate so the
    // shell (and its accounts surface) can read it.
    final client = getIt<FfiClient>();
    return tester.pumpWidget(
      BlocProvider<AppEventsBloc>(
        create: (_) => AppEventsBloc(client.events()),
        child: MaterialApp(
          theme: buildTheme(Brightness.light),
          home: const AccountGate(),
        ),
      ),
    );
  }

  testWidgets('zero accounts run the wizard first', (tester) async {
    configureDependencies(client: FakeFfiClient(accounts: const []));
    await pumpGate(tester);
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('wizard-welcome')), findsOneWidget);
  });

  testWidgets('existing accounts go straight to the shell', (tester) async {
    configureDependencies(
      client: FakeFfiClient(
        accounts: const [_workAccount],
        status: const DaemonStatus(
          state: DaemonState.running,
          daemonVersion: '0.4.0',
          protocolVersion: 1,
        ),
      ),
    );
    await pumpGate(tester);
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('shell-sidebar')), findsOneWidget);
    expect(find.byKey(const Key('wizard-welcome')), findsNothing);
  });

  testWidgets('a listing failure still shows the shell and its banner', (
    tester,
  ) async {
    configureDependencies(
      client: FakeFfiClient(
        accountsFailure: const DaemonUnavailableFailure(
          message: 'cannot reach the daemon',
        ),
        startFailure: const DaemonUnavailableFailure(),
      ),
    );
    await pumpGate(tester);
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('shell-sidebar')), findsOneWidget);
    expect(find.text('Sync paused — daemon unavailable'), findsOneWidget);
  });

  testWidgets('finishing the wizard swaps in the shell', (tester) async {
    final client = FakeFfiClient(accounts: const []);
    configureDependencies(client: client, urlOpener: FakeUrlOpener());
    await pumpGate(tester);
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('wizard-welcome')), findsOneWidget);

    // The account now exists behind the daemon; finishing the wizard must
    // re-check and swap the shell in (the connected view is transient:
    // the gate replaces the wizard subtree as soon as it hears onDone).
    client.accounts = const [_workAccount];
    final wizardElement = tester.element(
      find.byKey(const Key('wizard-welcome')),
    );
    wizardElement.read<AccountWizardBloc>().add(
      const ImapAccountSaveRequested(draft: _workDraft, password: 'pw'),
    );
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('wizard-welcome')), findsNothing);
    expect(find.byKey(const Key('shell-sidebar')), findsOneWidget);
    // Gate re-check + the shell's accounts surface loading its list.
    expect(client.listAccountsCalls, 3);
  });
}

const _workDraft = ImapDraft(
  email: 'work@example.com',
  imapHost: 'imap.example.com',
  imapPort: 993,
  imapSecurity: 'tls',
  smtpHost: 'smtp.example.com',
  smtpPort: 465,
  smtpSecurity: 'tls',
);
