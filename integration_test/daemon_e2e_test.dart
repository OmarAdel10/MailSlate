import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:mailslate/main.dart' as app;

/// Boots the real GUI against the real Rust bridge and a real spawned
/// daemon binary (MAILSLATE-19).
///
/// The harness must provide through the environment:
///
/// - `MAILSLATE_DAEMON_PATH`: a built `mailslate-daemon` binary
///   (`rust/target/debug/mailslate-daemon`; the e2e CI job builds it)
/// - `MAILSLATE_DATA_DIR`: a scratch data directory so the run never
///   touches a real profile. It must start empty: the startup gate only
///   reaches the wizard when the daemon reports zero accounts.
///
/// Shutdown is not wired yet — a local run leaves the spawned daemon and
/// the scratch directory behind (MAILSLATE-32).
///
/// See the `e2e` job in `.github/workflows/ci.yml` for the wired-up
/// example.
void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('first launch on an empty profile lands in the account wizard', (
    tester,
  ) async {
    app.main(const []);

    // Boot: bridge init spawns the daemon, the gate lists accounts over
    // FRB, and zero accounts route to the wizard (USER_FLOW §3). Cold
    // daemon spawns on a slow disk need a generous budget.
    await _pumpUntilFound(
      tester,
      find.byKey(const Key('wizard-welcome')),
      const Duration(seconds: 90),
    );

    // The shell must stay unmounted behind the gate on an empty profile.
    expect(find.byKey(const Key('shell-sidebar')), findsNothing);

    // Step 1 is live: "Connect account" is enabled, offline is not.
    final connect = find.byKey(const Key('wizard-connect-account'));
    expect(tester.widget<FilledButton>(connect).onPressed, isNotNull);
    expect(
      tester
          .widget<OutlinedButton>(
            find.byKey(const Key('wizard-explore-offline')),
          )
          .onPressed,
      isNull,
    );

    // Step 2 renders the three provider cards.
    await tester.tap(connect);
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('wizard-card-gmail')), findsOneWidget);
    expect(find.byKey(const Key('wizard-card-outlook')), findsOneWidget);
    expect(find.byKey(const Key('wizard-card-other')), findsOneWidget);

    // Step 3: the other-IMAP form renders without touching the network.
    await tester.tap(find.byKey(const Key('wizard-card-other')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('wizard-field-email')), findsOneWidget);
    expect(find.byKey(const Key('wizard-test-connection')), findsOneWidget);
  }, timeout: const Timeout(Duration(minutes: 3)));
}

/// Pumps on a fixed cadence until [target] renders or [budget] elapses.
///
/// `pumpAndSettle` cannot be used for boot: the daemon connects on its own
/// schedule, not in response to a frame.
Future<void> _pumpUntilFound(
  WidgetTester tester,
  Finder target,
  Duration budget,
) async {
  const step = Duration(milliseconds: 250);
  final deadline = DateTime.now().add(budget);
  while (target.evaluate().isEmpty && DateTime.now().isBefore(deadline)) {
    await tester.pump(step);
  }
  expect(
    target,
    findsOneWidget,
    reason: 'account wizard never rendered against the spawned daemon',
  );
}
