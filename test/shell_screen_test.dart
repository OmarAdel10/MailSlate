import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mailslate/core/di/di.dart';
import 'package:mailslate/core/failures/failures.dart';
import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/core/theme/theme.dart';
import 'package:mailslate/features/shell/shell_screen.dart';

import 'support/fake_ffi_client.dart';

const _running = DaemonStatus(
  state: DaemonState.running,
  daemonVersion: '0.9.0',
  protocolVersion: 1,
);

void main() {
  setUp(getIt.reset);

  Widget shell() => MaterialApp(
    theme: buildTheme(Brightness.light),
    home: const ShellScreen(),
  );

  testWidgets('renders the three panes with spec sizes', (tester) async {
    configureDependencies(client: FakeFfiClient(status: _running));

    await tester.pumpWidget(shell());
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
    configureDependencies(client: FakeFfiClient(status: _running));

    await tester.pumpWidget(shell());
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

    await tester.pumpWidget(shell());
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
    configureDependencies(
      client: FakeFfiClient(
        status: const DaemonStatus(
          state: DaemonState.running,
          protocolVersion: 1,
        ),
      ),
    );

    await tester.pumpWidget(shell());
    await tester.pumpAndSettle();

    expect(find.text('Daemon connected'), findsOneWidget);
    expect(find.textContaining('Daemon v'), findsNothing);
  });

  testWidgets('shows the paused banner for unexpected status errors', (
    tester,
  ) async {
    configureDependencies(client: FakeFfiClient(statusThrow: StateError('x')));

    await tester.pumpWidget(shell());
    await tester.pumpAndSettle();

    expect(find.text('Sync paused — daemon unavailable'), findsOneWidget);
    expect(find.text('Retry'), findsOneWidget);
  });

  testWidgets('re-checks the daemon when the refresh button is tapped', (
    tester,
  ) async {
    final client = FakeFfiClient(status: _running);
    configureDependencies(client: client);

    await tester.pumpWidget(shell());
    await tester.pumpAndSettle();

    expect(client.statusCalls, 1);

    await tester.tap(find.byTooltip('Check daemon'));
    await tester.pumpAndSettle();

    expect(client.statusCalls, 2);
  });
}
