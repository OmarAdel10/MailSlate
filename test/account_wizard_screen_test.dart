import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mailslate/core/di/di.dart';
import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/core/theme/theme.dart';
import 'package:mailslate/features/accounts/wizard/account_wizard_screen.dart';

import 'support/fake_ffi_client.dart';
import 'support/fake_url_opener.dart';

void main() {
  setUp(getIt.reset);

  Future<Widget> pumpWizard(
    WidgetTester tester, {
    FakeFfiClient? client,
    FakeUrlOpener? urlOpener,
    bool firstAccount = true,
    VoidCallback? onDone,
  }) async {
    final resolvedClient = client ?? FakeFfiClient();
    final resolvedOpener = urlOpener ?? FakeUrlOpener();
    configureDependencies(client: resolvedClient, urlOpener: resolvedOpener);
    await tester.pumpWidget(
      MaterialApp(
        theme: buildTheme(Brightness.light),
        home: AccountWizardScreen(firstAccount: firstAccount, onDone: onDone),
      ),
    );
    await tester.pumpAndSettle();
    return tester.widget(find.byType(AccountWizardScreen));
  }

  testWidgets('welcomes a first-run user', (tester) async {
    await pumpWizard(tester);

    expect(find.text('Welcome to MailSlate'), findsOneWidget);
    expect(find.byKey(const Key('wizard-connect-account')), findsOneWidget);
    // No data exists before the first account: Explore offline is disabled.
    final explore = tester.widget<OutlinedButton>(
      find.byKey(const Key('wizard-explore-offline')),
    );
    expect(explore.onPressed, isNull);
  });

  testWidgets('picks gmail and waits for the browser', (tester) async {
    final client = FakeFfiClient();
    final opener = FakeUrlOpener();
    await pumpWizard(tester, client: client, urlOpener: opener);

    await tester.tap(find.byKey(const Key('wizard-connect-account')));
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const Key('wizard-card-gmail')));
    await tester.pumpAndSettle();

    expect(find.text('Complete sign-in in your browser…'), findsOneWidget);
    expect(find.byKey(const Key('wizard-reopen-browser')), findsOneWidget);
    expect(opener.openedUrls, [client.oauthUrl]);
  });

  testWidgets('second accounts skip the welcome step', (tester) async {
    await pumpWizard(tester, firstAccount: false);

    expect(find.text('Welcome to MailSlate'), findsNothing);
    expect(find.byKey(const Key('wizard-card-gmail')), findsOneWidget);
  });

  testWidgets('shows the imap form for other providers', (tester) async {
    await pumpWizard(tester, firstAccount: false);

    await tester.tap(find.byKey(const Key('wizard-card-other')));
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('wizard-imap-form')), findsOneWidget);
    expect(find.byKey(const Key('wizard-field-email')), findsOneWidget);
    expect(find.byKey(const Key('wizard-field-password')), findsOneWidget);
    expect(find.byKey(const Key('wizard-field-imap-host')), findsOneWidget);
    expect(find.byKey(const Key('wizard-field-imap-port')), findsOneWidget);
  });

  testWidgets('fills the preset hosts when a preset is tapped', (tester) async {
    await pumpWizard(tester, firstAccount: false);
    await tester.tap(find.byKey(const Key('wizard-card-other')));
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const Key('wizard-smtp-section')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('wizard-preset-Yahoo')));
    await tester.pumpAndSettle();

    TextField hostField() => tester.widget<TextField>(
      find.descendant(
        of: find.byKey(const Key('wizard-field-imap-host')),
        matching: find.byType(TextField),
      ),
    );
    TextField smtpField() => tester.widget<TextField>(
      find.descendant(
        of: find.byKey(const Key('wizard-field-smtp-host')),
        matching: find.byType(TextField),
      ),
    );
    expect(hostField().controller!.text, 'imap.mail.yahoo.com');
    expect(smtpField().controller!.text, 'smtp.mail.yahoo.com');
  });

  testWidgets('tests the connection and enables save on success', (
    tester,
  ) async {
    await pumpWizard(tester, firstAccount: false);
    await tester.tap(find.byKey(const Key('wizard-card-other')));
    await tester.pumpAndSettle();

    await tester.enterText(
      find.byKey(const Key('wizard-field-email')),
      'work@example.com',
    );
    await tester.enterText(
      find.byKey(const Key('wizard-field-password')),
      'secret-pw',
    );
    await tester.enterText(
      find.byKey(const Key('wizard-field-imap-host')),
      'imap.example.com',
    );
    await tester.enterText(
      find.byKey(const Key('wizard-field-imap-port')),
      '993',
    );
    await tester.tap(find.byKey(const Key('wizard-smtp-section')));
    await tester.pumpAndSettle();
    await tester.enterText(
      find.byKey(const Key('wizard-field-smtp-host')),
      'smtp.example.com',
    );
    await tester.enterText(
      find.byKey(const Key('wizard-field-smtp-port')),
      '465',
    );

    await tester.pumpAndSettle();
    await tester.ensureVisible(find.byKey(const Key('wizard-test-connection')));
    await tester.tap(find.byKey(const Key('wizard-test-connection')));
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('wizard-imap-step')), findsOneWidget);
    expect(find.byKey(const Key('wizard-smtp-step')), findsOneWidget);
    final add = tester.widget<FilledButton>(
      find.byKey(const Key('wizard-add-account')),
    );
    expect(add.onPressed, isNotNull);
  });

  testWidgets('keeps the form open when a step fails', (tester) async {
    final client = FakeFfiClient(
      testConnectionResult: const ConnectionTest(
        imap: ConnectionStep(ok: false, error: 'auth failed'),
        smtp: ConnectionStep(ok: true),
      ),
    );
    await pumpWizard(tester, client: client, firstAccount: false);
    await tester.tap(find.byKey(const Key('wizard-card-other')));
    await tester.pumpAndSettle();

    await tester.enterText(
      find.byKey(const Key('wizard-field-email')),
      'work@example.com',
    );
    await tester.enterText(
      find.byKey(const Key('wizard-field-password')),
      'secret-pw',
    );
    await tester.enterText(
      find.byKey(const Key('wizard-field-imap-host')),
      'imap.example.com',
    );
    await tester.enterText(
      find.byKey(const Key('wizard-field-imap-port')),
      '993',
    );
    await tester.tap(find.byKey(const Key('wizard-smtp-section')));
    await tester.pumpAndSettle();
    await tester.enterText(
      find.byKey(const Key('wizard-field-smtp-host')),
      'smtp.example.com',
    );
    await tester.enterText(
      find.byKey(const Key('wizard-field-smtp-port')),
      '465',
    );

    await tester.pumpAndSettle();
    await tester.ensureVisible(find.byKey(const Key('wizard-test-connection')));
    await tester.tap(find.byKey(const Key('wizard-test-connection')));
    await tester.pumpAndSettle();

    // The failing step is expanded with its error; save stays disabled.
    expect(find.text('auth failed'), findsOneWidget);
    final add = tester.widget<FilledButton>(
      find.byKey(const Key('wizard-add-account')),
    );
    expect(add.onPressed, isNull);
  });

  testWidgets('connects an imap account end to end', (tester) async {
    final client = FakeFfiClient();
    await pumpWizard(tester, client: client, firstAccount: false);
    await tester.tap(find.byKey(const Key('wizard-card-other')));
    await tester.pumpAndSettle();

    await tester.enterText(
      find.byKey(const Key('wizard-field-email')),
      'work@example.com',
    );
    await tester.enterText(
      find.byKey(const Key('wizard-field-password')),
      'secret-pw',
    );
    await tester.enterText(
      find.byKey(const Key('wizard-field-imap-host')),
      'imap.example.com',
    );
    await tester.enterText(
      find.byKey(const Key('wizard-field-imap-port')),
      '993',
    );
    await tester.tap(find.byKey(const Key('wizard-smtp-section')));
    await tester.pumpAndSettle();
    await tester.enterText(
      find.byKey(const Key('wizard-field-smtp-host')),
      'smtp.example.com',
    );
    await tester.enterText(
      find.byKey(const Key('wizard-field-smtp-port')),
      '465',
    );
    await tester.pumpAndSettle();
    await tester.ensureVisible(find.byKey(const Key('wizard-test-connection')));
    await tester.tap(find.byKey(const Key('wizard-test-connection')));
    await tester.pumpAndSettle();

    await tester.ensureVisible(find.byKey(const Key('wizard-add-account')));
    await tester.tap(find.byKey(const Key('wizard-add-account')));
    await tester.pumpAndSettle();

    expect(find.text('Account connected'), findsOneWidget);
    expect(client.addImapCalls, 1);
    expect(client.lastAddDraft!.email, 'work@example.com');
    expect(client.lastAddPassword, 'secret-pw');
  });

  testWidgets('reports the connected account via onDone', (tester) async {
    var done = false;
    final oauthEvents = StreamController<AppEvent>.broadcast();
    final client = FakeFfiClient()..eventStream = () => oauthEvents.stream;
    await pumpWizard(
      tester,
      client: client,
      firstAccount: false,
      onDone: () => done = true,
    );

    await tester.tap(find.byKey(const Key('wizard-card-gmail')));
    await tester.pumpAndSettle();

    oauthEvents.add(const AppEvent.oauthCompleted(email: 'user@gmail.com'));
    await tester.pumpAndSettle();

    expect(find.text('Account connected'), findsOneWidget);
    expect(done, isTrue);
    await oauthEvents.close();
  });
}
