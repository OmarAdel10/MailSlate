import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_bloc/flutter_bloc.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mailslate/core/di/di.dart';
import 'package:mailslate/core/failures/failures.dart';
import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/core/theme/theme.dart';
import 'package:mailslate/features/shell/app_events_bloc.dart';
import 'package:mailslate/features/shell/shell_screen.dart';

import 'support/fake_ffi_client.dart';
import 'support/fake_log_opener.dart';
import 'support/fake_url_opener.dart';

const _running = DaemonStatus(
  state: DaemonState.running,
  daemonVersion: '0.9.0',
  protocolVersion: 1,
);

/// Minimal oauth account fixture for the sidebar rows.
AccountSummary _account(
  int id,
  String name, {
  String email = 'user@example.com',
  String? color,
}) {
  return AccountSummary(
    id: id,
    displayName: name,
    email: email,
    provider: 'gmail',
    authType: 'oauth2',
    color: color,
    syncWindowDays: 365,
  );
}

void main() {
  setUp(getIt.reset);

  Widget shell(FakeFfiClient client) => BlocProvider<AppEventsBloc>(
    create: (_) => AppEventsBloc(client.eventStream()),
    child: MaterialApp(
      theme: buildTheme(Brightness.light),
      home: const ShellScreen(),
    ),
  );

  testWidgets('renders the three panes with spec sizes', (tester) async {
    final client = FakeFfiClient(status: _running);
    configureDependencies(client: client);

    await tester.pumpWidget(shell(client));
    await tester.pumpAndSettle();

    expect(tester.getSize(find.byKey(const Key('shell-sidebar'))).width, 240);
    expect(tester.getSize(find.byKey(const Key('shell-toolbar'))).height, 56);
    expect(tester.getSize(find.byKey(const Key('shell-list'))).width, 380);
    expect(tester.getSize(find.byKey(const Key('shell-reader'))).width, 178);

    for (final label in ['Starred', 'Drafts', 'Sent', 'Spam', 'Trash']) {
      expect(find.text(label), findsOneWidget, reason: label);
    }
    // Important is hidden until a connected account exposes it (M2).
    expect(find.text('Important'), findsNothing);

    expect(find.text('Inbox is empty'), findsOneWidget);
    expect(find.text('Select a conversation'), findsOneWidget);
    expect(find.text('Daemon v0.9.0'), findsOneWidget);
  });

  testWidgets('switches the list empty state with the selected view', (
    tester,
  ) async {
    final client = FakeFfiClient(status: _running);
    configureDependencies(client: client);

    await tester.pumpWidget(shell(client));
    await tester.pumpAndSettle();

    await tester.tap(find.text('Trash'));
    await tester.pumpAndSettle();

    expect(find.text('Trash is empty'), findsOneWidget);
    expect(find.text('Inbox is empty'), findsNothing);
  });

  testWidgets('shows the paused banner and recovers on retry', (tester) async {
    final client = FakeFfiClient(
      startFailure: const DaemonUnavailableFailure(),
    );
    configureDependencies(client: client);

    await tester.pumpWidget(shell(client));
    await tester.pumpAndSettle();

    expect(find.text('Sync paused — daemon unavailable'), findsOneWidget);
    expect(find.text('Sync paused'), findsOneWidget);
    expect(find.text('Retry'), findsOneWidget);

    client.startFailure = null;
    client.pingResult = const PingInfo(
      protocolVersion: 1,
      daemonVersion: '0.9.0',
    );

    await tester.tap(find.text('Retry'));
    await tester.pumpAndSettle();

    expect(find.text('Sync paused — daemon unavailable'), findsNothing);
    expect(find.text('Retry'), findsNothing);
    expect(find.text('Daemon v0.9.0'), findsOneWidget);
  });

  testWidgets('labels a connected daemon without a version', (tester) async {
    final client = FakeFfiClient(
      status: const DaemonStatus(
        state: DaemonState.running,
        protocolVersion: 1,
      ),
    );
    configureDependencies(client: client);

    await tester.pumpWidget(shell(client));
    await tester.pumpAndSettle();

    expect(find.text('Daemon connected'), findsOneWidget);
    expect(find.textContaining('Daemon v'), findsNothing);
  });

  testWidgets('shows the paused banner for unexpected status errors', (
    tester,
  ) async {
    final client = FakeFfiClient(statusThrow: StateError('x'));
    configureDependencies(client: client);

    await tester.pumpWidget(shell(client));
    await tester.pumpAndSettle();

    expect(find.text('Sync paused — daemon unavailable'), findsOneWidget);
    expect(find.text('Retry'), findsOneWidget);
  });

  testWidgets('re-checks the daemon when the refresh button is tapped', (
    tester,
  ) async {
    final client = FakeFfiClient(status: _running);
    configureDependencies(client: client);

    await tester.pumpWidget(shell(client));
    await tester.pumpAndSettle();

    expect(client.statusCalls, 1);

    await tester.tap(find.byTooltip('Check daemon'));
    await tester.pumpAndSettle();

    expect(client.statusCalls, 2);
  });

  testWidgets('paused banner opens the newest daemon log', (tester) async {
    final logDir = Directory.systemTemp.createTempSync('mailslate-ui-log-');
    addTearDown(() => logDir.deleteSync(recursive: true));
    final older = File('${logDir.path}/daemon.2026-10-15.log');
    final newer = File('${logDir.path}/daemon.2026-10-16.log');
    older.writeAsStringSync('older');
    newer.writeAsStringSync('newer');
    // Pin the modification times: the newest-log lookup sorts by mtime,
    // and back-to-back writes can share a timestamp on coarse clocks.
    older.setLastModifiedSync(DateTime.parse('2026-10-15'));
    newer.setLastModifiedSync(DateTime.parse('2026-10-16'));
    final opener = FakeLogOpener();
    final client = FakeFfiClient(
      startFailure: const DaemonUnavailableFailure(),
      status: const DaemonStatus(state: DaemonState.unavailable),
      logDir: logDir.path,
    );
    configureDependencies(client: client, logOpener: opener);

    await tester.pumpWidget(shell(client));
    await tester.pumpAndSettle();

    await tester.tap(find.text('View log'));
    await tester.pumpAndSettle();

    expect(opener.paths, [newer.path]);
  });

  testWidgets('View log stays disabled while no log exists', (tester) async {
    final logDir = Directory.systemTemp.createTempSync('mailslate-ui-log-');
    addTearDown(() => logDir.deleteSync(recursive: true));
    final opener = FakeLogOpener();
    final client = FakeFfiClient(
      startFailure: const DaemonUnavailableFailure(),
      status: const DaemonStatus(state: DaemonState.unavailable),
      logDir: logDir.path,
    );
    configureDependencies(client: client, logOpener: opener);

    await tester.pumpWidget(shell(client));
    await tester.pumpAndSettle();

    final button = tester.widget<TextButton>(
      find.byKey(const Key('view-log-button')),
    );
    expect(button.onPressed, isNull);
    expect(find.byTooltip('No log file yet'), findsOneWidget);
    expect(opener.paths, isEmpty);
  });

  testWidgets('the sidebar offers adding another account', (tester) async {
    final client = FakeFfiClient(status: _running);
    configureDependencies(client: client, urlOpener: FakeUrlOpener());

    await tester.pumpWidget(shell(client));
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('sidebar-add-account')), findsOneWidget);

    await tester.tap(find.byKey(const Key('sidebar-add-account')));
    await tester.pumpAndSettle();

    // Additional accounts skip the welcome step (USER_FLOW §3).
    expect(find.byKey(const Key('wizard-welcome')), findsNothing);
    expect(find.text('Connect your mail'), findsOneWidget);
  });

  testWidgets('the sidebar lists the connected accounts', (tester) async {
    final client = FakeFfiClient(
      status: _running,
      accounts: [
        _account(1, 'Personal', email: 'personal@example.com'),
        _account(2, 'Work', email: 'work@example.com'),
      ],
    );
    configureDependencies(client: client);

    await tester.pumpWidget(shell(client));
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('account-row-1')), findsOneWidget);
    expect(find.byKey(const Key('account-row-2')), findsOneWidget);
    expect(find.text('Personal'), findsOneWidget);
    expect(find.text('Work'), findsOneWidget);
    expect(find.text('Accounts'), findsOneWidget);
    // No accounts flagged → no reconnect cards anywhere.
    expect(find.textContaining('Reconnect'), findsNothing);
  });

  testWidgets('tapping an account row opens its settings dialog', (
    tester,
  ) async {
    final client = FakeFfiClient(
      status: _running,
      accounts: [_account(1, 'Personal')],
    );
    configureDependencies(client: client);

    await tester.pumpWidget(shell(client));
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const Key('account-row-1')));
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('account-settings-dialog')), findsOneWidget);
  });

  testWidgets('saving settings patches the account and reloads', (
    tester,
  ) async {
    final client = FakeFfiClient(
      status: _running,
      accounts: [_account(1, 'Personal')],
    );
    configureDependencies(client: client);

    await tester.pumpWidget(shell(client));
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const Key('account-row-1')));
    await tester.pumpAndSettle();

    await tester.enterText(
      find.byKey(const Key('account-name-field')),
      'Renamed',
    );
    await tester.tap(find.byKey(const Key('account-save')));
    await tester.pumpAndSettle();

    expect(client.updateCalls, 1);
    expect(client.lastUpdateId, 1);
    expect(client.lastPatch?.displayName, 'Renamed');
    // The dialog popped and the bloc reloaded the list.
    expect(find.byKey(const Key('account-settings-dialog')), findsNothing);
    expect(client.listAccountsCalls, greaterThan(1));
  });

  testWidgets('an empty display name is rejected inline', (tester) async {
    final client = FakeFfiClient(
      status: _running,
      accounts: [_account(1, 'Personal')],
    );
    configureDependencies(client: client);

    await tester.pumpWidget(shell(client));
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const Key('account-row-1')));
    await tester.pumpAndSettle();

    await tester.enterText(find.byKey(const Key('account-name-field')), '  ');
    await tester.tap(find.byKey(const Key('account-save')));
    await tester.pumpAndSettle();

    expect(find.text('Display name cannot be empty'), findsOneWidget);
    expect(client.updateCalls, 0);
    expect(find.byKey(const Key('account-settings-dialog')), findsOneWidget);
  });

  testWidgets('picking an accent color patches color', (tester) async {
    final client = FakeFfiClient(
      status: _running,
      accounts: [_account(1, 'Personal')],
    );
    configureDependencies(client: client);

    await tester.pumpWidget(shell(client));
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const Key('account-row-1')));
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const Key('account-color-#EC4899')));
    await tester.tap(find.byKey(const Key('account-save')));
    await tester.pumpAndSettle();

    expect(client.lastPatch?.color, '#EC4899');
    expect(client.lastPatch?.clearColor, isFalse);
  });

  testWidgets('the none swatch clears a stored accent color', (tester) async {
    final client = FakeFfiClient(
      status: _running,
      accounts: [_account(1, 'Personal', color: '#FB923C')],
    );
    configureDependencies(client: client);

    await tester.pumpWidget(shell(client));
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const Key('account-row-1')));
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const Key('account-color-none')));
    await tester.tap(find.byKey(const Key('account-save')));
    await tester.pumpAndSettle();

    expect(client.lastPatch?.color, isNull);
    expect(client.lastPatch?.clearColor, isTrue);
  });

  testWidgets('the sync window dropdown patches the stored value', (
    tester,
  ) async {
    final client = FakeFfiClient(
      status: _running,
      accounts: [_account(1, 'Personal')],
    );
    configureDependencies(client: client);

    await tester.pumpWidget(shell(client));
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const Key('account-row-1')));
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const Key('account-sync-window')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('90 days').last);
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const Key('account-save')));
    await tester.pumpAndSettle();

    expect(client.lastPatch?.syncWindowDays, 90);
  });

  testWidgets('remove asks for confirmation, then removes and reloads', (
    tester,
  ) async {
    final client = FakeFfiClient(
      status: _running,
      accounts: [_account(1, 'Personal')],
    );
    configureDependencies(client: client);

    await tester.pumpWidget(shell(client));
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const Key('account-row-1')));
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const Key('account-remove')));
    await tester.pumpAndSettle();

    expect(
      find.byKey(const Key('account-remove-confirmation')),
      findsOneWidget,
    );
    expect(client.removeCalls, 0);

    // Removal empties the account list the reload will fetch.
    client.accounts = [];

    await tester.tap(find.byKey(const Key('account-remove-confirm')));
    await tester.pumpAndSettle();

    expect(client.removeCalls, 1);
    expect(client.lastRemoveId, 1);
    expect(find.byKey(const Key('account-remove-confirmation')), findsNothing);
    expect(find.byKey(const Key('account-row-1')), findsNothing);
  });

  testWidgets('a reauth_required account raises a reconnect card', (
    tester,
  ) async {
    final events = StreamController<AppEvent>();
    addTearDown(events.close);
    final client = FakeFfiClient(
      status: _running,
      accounts: [_account(7, 'Gmail')],
      eventStream: () => events.stream,
    );
    configureDependencies(client: client, urlOpener: FakeUrlOpener());

    await tester.pumpWidget(shell(client));
    await tester.pumpAndSettle();

    events.add(
      const AppEvent.accountStatus(account: 7, state: 'reauth_required'),
    );
    await tester.pumpAndSettle();

    expect(find.text('Reconnect Gmail'), findsOneWidget);
    expect(find.byKey(const Key('account-reconnect-7')), findsOneWidget);
    // The sidebar row flags the account too (USER_FLOW §12).
    expect(find.byTooltip('Reconnect needed'), findsOneWidget);
  });

  testWidgets('tapping reconnect starts the reauth flow in the browser', (
    tester,
  ) async {
    final events = StreamController<AppEvent>();
    addTearDown(events.close);
    final opener = FakeUrlOpener();
    final client = FakeFfiClient(
      status: _running,
      accounts: [_account(7, 'Gmail')],
      eventStream: () => events.stream,
    );
    configureDependencies(client: client, urlOpener: opener);

    await tester.pumpWidget(shell(client));
    await tester.pumpAndSettle();

    events.add(
      const AppEvent.accountStatus(account: 7, state: 'reauth_required'),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const Key('account-reconnect-7')));
    // The card shows an indeterminate spinner while the flow is
    // pending — pumpAndSettle would never settle, so advance frames.
    await tester.pump();
    await tester.pump();

    expect(client.reauthCalls, 1);
    expect(client.lastReauthId, 7);
    expect(opener.openedUrls, [client.reauthUrl]);
    expect(find.text('Waiting for the browser…'), findsOneWidget);
    // One flow at a time daemon-wide: the card disables while pending.
    final button = tester.widget<FilledButton>(
      find.byKey(const Key('account-reconnect-7')),
    );
    expect(button.onPressed, isNull);
  });

  testWidgets('oauth_reconnected clears the card and pending state', (
    tester,
  ) async {
    final events = StreamController<AppEvent>();
    addTearDown(events.close);
    final client = FakeFfiClient(
      status: _running,
      accounts: [_account(7, 'Gmail')],
      eventStream: () => events.stream,
    );
    configureDependencies(client: client, urlOpener: FakeUrlOpener());

    await tester.pumpWidget(shell(client));
    await tester.pumpAndSettle();

    events.add(
      const AppEvent.accountStatus(account: 7, state: 'reauth_required'),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const Key('account-reconnect-7')));
    // The card shows an indeterminate spinner while the flow is
    // pending — pumpAndSettle would never settle, so advance frames.
    await tester.pump();
    await tester.pump();

    events.add(
      const AppEvent.oauthReconnected(account: 7, email: 'user@example.com'),
    );
    await tester.pumpAndSettle();

    expect(find.text('Reconnect Gmail'), findsNothing);
    expect(find.byTooltip('Reconnect needed'), findsNothing);
    expect(find.text('Waiting for the browser…'), findsNothing);
  });

  testWidgets('a failed reconnect shows the reason on the card', (
    tester,
  ) async {
    final events = StreamController<AppEvent>();
    addTearDown(events.close);
    final client = FakeFfiClient(
      status: _running,
      accounts: [_account(7, 'Gmail')],
      eventStream: () => events.stream,
    );
    configureDependencies(client: client, urlOpener: FakeUrlOpener());

    await tester.pumpWidget(shell(client));
    await tester.pumpAndSettle();

    events.add(
      const AppEvent.accountStatus(account: 7, state: 'reauth_required'),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const Key('account-reconnect-7')));
    // The card shows an indeterminate spinner while the flow is
    // pending — pumpAndSettle would never settle, so advance frames.
    await tester.pump();
    await tester.pump();

    events.add(const AppEvent.oauthFailed(reason: 'token exchange rejected'));
    await tester.pumpAndSettle();

    expect(find.text('token exchange rejected'), findsOneWidget);
    expect(find.text('Waiting for the browser…'), findsNothing);
    // Failed flow clears pending: the card is tappable again.
    final button = tester.widget<FilledButton>(
      find.byKey(const Key('account-reconnect-7')),
    );
    expect(button.onPressed, isNotNull);
  });

  testWidgets('a reauth flow that cannot open the browser shows an error', (
    tester,
  ) async {
    final events = StreamController<AppEvent>();
    addTearDown(events.close);
    final opener = FakeUrlOpener(openResult: false);
    final client = FakeFfiClient(
      status: _running,
      accounts: [_account(7, 'Gmail')],
      eventStream: () => events.stream,
    );
    configureDependencies(client: client, urlOpener: opener);

    await tester.pumpWidget(shell(client));
    await tester.pumpAndSettle();

    events.add(
      const AppEvent.accountStatus(account: 7, state: 'reauth_required'),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const Key('account-reconnect-7')));
    // The card shows an indeterminate spinner while the flow is
    // pending — pumpAndSettle would never settle, so advance frames.
    await tester.pump();
    await tester.pump();

    expect(find.text('Could not open the browser'), findsOneWidget);
    final button = tester.widget<FilledButton>(
      find.byKey(const Key('account-reconnect-7')),
    );
    expect(button.onPressed, isNotNull);
  });

  testWidgets('Ctrl+F opens the search screen', (tester) async {
    final client = FakeFfiClient(status: _running);
    configureDependencies(client: client);

    await tester.pumpWidget(shell(client));
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('shell-list')), findsOneWidget);
    expect(find.byKey(const Key('shell-search-screen')), findsNothing);

    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.keyF);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.keyF);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('shell-search-screen')), findsOneWidget);
    expect(find.byKey(const Key('shell-list')), findsNothing);
  });

  testWidgets('Cmd+F (meta) also opens the search screen', (tester) async {
    final client = FakeFfiClient(status: _running);
    configureDependencies(client: client);

    await tester.pumpWidget(shell(client));
    await tester.pumpAndSettle();

    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.keyF);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.keyF);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('shell-search-screen')), findsOneWidget);
    expect(find.byKey(const Key('shell-list')), findsNothing);
  });

  testWidgets('Esc returns to the list while search is open', (tester) async {
    final client = FakeFfiClient(status: _running);
    configureDependencies(client: client);

    await tester.pumpWidget(shell(client));
    await tester.pumpAndSettle();

    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.keyF);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.keyF);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('shell-search-screen')), findsOneWidget);

    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('shell-list')), findsOneWidget);
    expect(find.byKey(const Key('shell-search-screen')), findsNothing);
  });

  testWidgets('Esc without search on does not change the view', (tester) async {
    final client = FakeFfiClient(status: _running);
    configureDependencies(client: client);

    await tester.pumpWidget(shell(client));
    await tester.pumpAndSettle();

    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('shell-list')), findsOneWidget);
  });

  testWidgets('the toolbar search button opens the search screen', (
    tester,
  ) async {
    final client = FakeFfiClient(status: _running);
    configureDependencies(client: client);

    await tester.pumpWidget(shell(client));
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const Key('shell-search')));
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('shell-search-screen')), findsOneWidget);
    expect(find.byKey(const Key('shell-list')), findsNothing);
  });

  testWidgets('the close button in search returns to the list', (tester) async {
    final client = FakeFfiClient(status: _running);
    configureDependencies(client: client);

    await tester.pumpWidget(shell(client));
    await tester.pumpAndSettle();

    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.keyF);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.keyF);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('shell-search-screen')), findsOneWidget);

    await tester.tap(find.byKey(const Key('search-close')));
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('shell-list')), findsOneWidget);
    expect(find.byKey(const Key('shell-search-screen')), findsNothing);
  });
}
