import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mailslate/core/di/di.dart';
import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/core/routing/launch_args.dart';
import 'package:mailslate/features/shell/shell_screen.dart';
import 'package:mailslate/main.dart' as app;

import 'support/fake_ffi_client.dart';

void main() {
  setUp(getIt.reset);
  tearDown(getIt.reset);

  testWidgets('boots the shell and forwards the open-thread launch arg', (
    tester,
  ) async {
    configureDependencies(
      client: FakeFfiClient(
        status: const DaemonStatus(
          state: DaemonState.running,
          daemonVersion: '0.4.0',
          protocolVersion: 1,
        ),
        accounts: const [
          AccountSummary(
            id: 1,
            displayName: 'Work',
            email: 'work@example.com',
            provider: 'imap',
            authType: 'password',
            syncWindowDays: 30,
          ),
        ],
      ),
    );

    await tester.pumpWidget(
      const app.MailSlateApp(launchArgs: LaunchArgs(openThreadId: 'thread-1')),
    );
    await tester.pumpAndSettle();

    final shell = tester.widget<ShellScreen>(find.byType(ShellScreen));
    expect(shell.openThreadId, 'thread-1');

    final materialApp = tester.widget<MaterialApp>(find.byType(MaterialApp));
    expect(materialApp.themeMode, ThemeMode.system);
    expect(materialApp.theme?.brightness, Brightness.light);
    expect(materialApp.darkTheme?.brightness, Brightness.dark);
  });

  test('configureDependencies falls back to the real client', () {
    configureDependencies();
    expect(getIt<FfiClient>(), isA<FfiClient>());
  });
}
