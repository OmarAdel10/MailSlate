import 'package:flutter/material.dart';
import 'package:flutter_bloc/flutter_bloc.dart';
import 'package:mailslate/core/di/di.dart';
import 'package:mailslate/core/ffi/client.dart';
import 'package:mailslate/core/ffi/events.dart';
import 'package:mailslate/core/logging/log.dart';
import 'package:mailslate/core/routing/launch_args.dart';
import 'package:mailslate/core/theme/theme.dart';
import 'package:mailslate/features/accounts/account_gate.dart';
import 'package:mailslate/features/shell/app_events_bloc.dart';

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

/// Root widget: theming (follows the system), the global event stream,
/// and the startup account gate.
class MailSlateApp extends StatelessWidget {
  const MailSlateApp({super.key, this.launchArgs = const LaunchArgs()});

  /// Arguments the window was opened with.
  final LaunchArgs launchArgs;

  @override
  Widget build(BuildContext context) {
    return BlocProvider(
      // The global events bloc merges the daemon stream into invalidation
      // signals (ARCHITECTURE §4); it subscribes nothing until a feature
      // listens.
      create: (_) => AppEventsBloc(appEventStream(getIt<FfiClient>())),
      child: MaterialApp(
        title: 'MailSlate',
        debugShowCheckedModeBanner: false,
        theme: buildTheme(Brightness.light),
        darkTheme: buildTheme(Brightness.dark),
        themeMode: ThemeMode.system,
        home: AccountGate(openThreadId: launchArgs.openThreadId),
      ),
    );
  }
}
