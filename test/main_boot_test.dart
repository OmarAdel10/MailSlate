import 'package:flutter_test/flutter_test.dart';
import 'package:mailslate/core/di/di.dart';
import 'package:mailslate/features/shell/shell_screen.dart';
import 'package:mailslate/main.dart' as app;

void main() {
  setUp(getIt.reset);
  tearDown(getIt.reset);

  // Lives in its own file: it boots the real `main()` once, and booting a
  // real frame after another pumped test can feed the connecting spinner a
  // stale frame timestamp (flutter AnimationController elapsed assertion).
  testWidgets('main survives a missing native bridge and still boots', (
    tester,
  ) async {
    // No dylib is available under flutter test, so initApp() fails and is
    // logged; the shell must still render with the daemon unavailable.
    await app.main(const []);
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 100));

    expect(find.byType(ShellScreen), findsOneWidget);
    expect(find.text('Sync paused — daemon unavailable'), findsOneWidget);
  });
}
