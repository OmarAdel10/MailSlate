import 'package:flutter/material.dart';
import 'package:mailslate/core/di/di.dart';
import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/core/logging/log.dart';
import 'package:mailslate/core/routing/launch_args.dart';
import 'package:mailslate/core/theme/theme.dart';
import 'package:mailslate/features/shell/shell_screen.dart';

Future<void> main(List<String> args) async {
  WidgetsFlutterBinding.ensureInitialized();

  final launchArgs = parseLaunchArgs(args);
  configureDependencies();

  try {
    await getIt<FfiClient>().initApp();
    logInfo('bridge initialized');
  } catch (error, stackTrace) {
    // The shell still renders and reports the daemon as unavailable.
    logError('bridge_init_failed', error: error, stackTrace: stackTrace);
  }

  runApp(MailSlateApp(launchArgs: launchArgs));
}

/// Root widget: theming (follows the system) plus the shell.
class MailSlateApp extends StatelessWidget {
  const MailSlateApp({super.key, this.launchArgs = const LaunchArgs()});

  /// Arguments the window was opened with.
  final LaunchArgs launchArgs;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'MailSlate',
      debugShowCheckedModeBanner: false,
      theme: buildTheme(Brightness.light),
      darkTheme: buildTheme(Brightness.dark),
      themeMode: ThemeMode.system,
      home: ShellScreen(openThreadId: launchArgs.openThreadId),
    );
  }
}
