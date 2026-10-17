import 'dart:io';

/// Finds the newest daemon rotation log inside [dir].
///
/// The daemon writes `daemon.<date>.log` files (daily rotation, seven kept);
/// the paused banner's View log action opens the newest one. Returns null
/// when [dir] is missing or holds no rotation file.
String? newestDaemonLog(String dir) {
  final directory = Directory(dir);
  if (!directory.existsSync()) {
    return null;
  }
  String? newestPath;
  DateTime? newestModified;
  for (final entity in directory.listSync()) {
    if (entity is! File) {
      continue;
    }
    final name = entity.uri.pathSegments.last;
    const prefix = 'daemon.';
    const suffix = '.log';
    if (name.length <= prefix.length + suffix.length ||
        !name.startsWith(prefix) ||
        !name.endsWith(suffix)) {
      continue;
    }
    final modified = entity.statSync().modified;
    if (newestModified == null || modified.isAfter(newestModified)) {
      newestModified = modified;
      newestPath = entity.path;
    }
  }
  return newestPath;
}
