/// Arguments the operating system passed when opening the window.
class LaunchArgs {
  const LaunchArgs({this.openThreadId});

  /// Conversation to focus on start, from `--open-thread <id>`.
  final String? openThreadId;
}

/// Parses a conversation to open from [args].
///
/// Accepts `--open-thread <id>` and `--open-thread=<id>`; anything else —
/// including a flag without a usable value — is ignored.
LaunchArgs parseLaunchArgs(List<String> args) {
  const flag = '--open-thread';
  const prefix = '$flag=';
  for (var i = 0; i < args.length; i++) {
    final arg = args[i];
    if (arg.startsWith(prefix)) {
      final value = arg.substring(prefix.length);
      if (value.isNotEmpty) {
        return LaunchArgs(openThreadId: value);
      }
      continue;
    }
    if (arg == flag && i + 1 < args.length) {
      final value = args[i + 1];
      if (value.isNotEmpty && !value.startsWith('-')) {
        return LaunchArgs(openThreadId: value);
      }
    }
  }
  return const LaunchArgs();
}
