import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mailslate/core/theme/theme.dart';
import 'package:mailslate/core/theme/tokens.dart';

void main() {
  testWidgets('theme follows the system brightness at runtime', (tester) async {
    final dispatcher = tester.view.platformDispatcher;
    dispatcher.platformBrightnessTestValue = Brightness.dark;
    addTearDown(dispatcher.clearPlatformBrightnessTestValue);

    late ThemeData theme;
    await tester.pumpWidget(
      MaterialApp(
        theme: buildTheme(Brightness.light),
        darkTheme: buildTheme(Brightness.dark),
        themeMode: ThemeMode.system,
        home: Builder(
          builder: (context) {
            theme = Theme.of(context);
            return const SizedBox.shrink();
          },
        ),
      ),
    );

    expect(theme.brightness, Brightness.dark);
    expect(theme.scaffoldBackgroundColor, AppTokens.dark.canvas);
    expect(theme.extension<AppTokens>(), same(AppTokens.dark));

    dispatcher.platformBrightnessTestValue = Brightness.light;
    await tester.pumpAndSettle();

    expect(theme.brightness, Brightness.light);
    expect(theme.scaffoldBackgroundColor, AppTokens.light.canvas);
    expect(theme.extension<AppTokens>(), same(AppTokens.light));
  });

  test('light and dark themes carry the DESIGN token sets', () {
    final light = buildTheme(Brightness.light);
    final dark = buildTheme(Brightness.dark);

    expect(light.extension<AppTokens>(), same(AppTokens.light));
    expect(dark.extension<AppTokens>(), same(AppTokens.dark));
    expect(light.colorScheme.primary, AppTokens.light.primary);
    expect(dark.colorScheme.surface, AppTokens.dark.canvas);
    expect(dark.scaffoldBackgroundColor, AppTokens.dark.canvas);
  });

  test('theme renders all UI text in Inter', () {
    final light = buildTheme(Brightness.light);
    final dark = buildTheme(Brightness.dark);

    expect(light.textTheme.bodyLarge?.fontFamily, 'Inter');
    expect(light.textTheme.titleSmall?.fontFamily, 'Inter');
    expect(light.textTheme.labelSmall?.fontFamily, 'Inter');
    expect(dark.textTheme.bodyMedium?.fontFamily, 'Inter');
    expect(dark.textTheme.displaySmall?.fontFamily, 'Inter');
  });

  test('excerpts use the JetBrains Mono family token', () {
    expect(kMonoFontFamily, 'JetBrainsMono');
  });

  test('copyWith replaces only the named tokens', () {
    final custom = AppTokens.dark.copyWith(primary: const Color(0xFF123456));
    expect(custom.primary, const Color(0xFF123456));
    expect(custom.canvas, AppTokens.dark.canvas);
    expect(custom.error, AppTokens.dark.error);

    final unchanged = AppTokens.light.copyWith();
    expect(unchanged.canvas, AppTokens.light.canvas);
    expect(unchanged.badgeEmerald, AppTokens.light.badgeEmerald);
  });
}
