import 'package:mailslate/core/system/url_opener.dart';

/// Test double for [UrlOpener]: records URLs, never spawns a process.
class FakeUrlOpener implements UrlOpener {
  FakeUrlOpener({this.openResult = true});

  /// Returned by every [open] call.
  bool openResult;

  /// Every URL passed to [open], in order.
  final List<String> openedUrls = [];

  @override
  Future<bool> open(String url) async {
    openedUrls.add(url);
    return openResult;
  }
}
