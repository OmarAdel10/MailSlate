import 'package:mailslate/core/logging/log_opener.dart';

/// Test double for [LogOpener]: records calls, never spawns processes.
class FakeLogOpener implements LogOpener {
  FakeLogOpener({this.result = true});

  /// Returned by [open].
  final bool result;

  /// Paths passed to [open], in call order.
  final List<String> paths = [];

  @override
  Future<bool> open(String path) async {
    paths.add(path);
    return result;
  }
}
