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
///   touches a real profile
///
/// See the `e2e` job in `.github/workflows/ci.yml` for the wired-up
/// example.
void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('GUI reaches the connected daemon status with a spawned daemon', (
    tester,
  ) async {
    app.main(const []);

    // The daemon spawns asynchronously and the shell shows a spinner
    // while connecting. Pump on a fixed cadence until the connected
    // label appears, with a generous budget for cold cargo caches.
    const step = Duration(milliseconds: 250);
    final deadline = DateTime.now().add(const Duration(seconds: 90));
    var connected = false;
    while (!connected && DateTime.now().isBefore(deadline)) {
      await tester.pump(step);
      connected = find
          .textContaining(RegExp('^Daemon (v|connected)'))
          .evaluate()
          .isNotEmpty;
    }
    expect(
      connected,
      isTrue,
      reason: 'daemon status never reached the connected state',
    );

    // The paused banner must not render while the daemon is live.
    expect(find.textContaining('Sync paused'), findsNothing);

    // The three-pane shell is up.
    expect(find.byKey(const Key('shell-sidebar')), findsOneWidget);
    expect(find.byKey(const Key('shell-list')), findsOneWidget);
    expect(find.byKey(const Key('shell-reader')), findsOneWidget);
  }, timeout: const Timeout(Duration(minutes: 3)));
}
