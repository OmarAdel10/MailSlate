import 'package:flutter/material.dart';

import 'tokens.dart';

/// Builds the app theme for [brightness] from the DESIGN.md token tables.
///
/// Light/dark are token swaps only — widgets never branch on brightness.
/// Typography uses the platform font stack in M1; bundling Inter and
/// JetBrains Mono is tracked as a follow-up (DESIGN §4).
ThemeData buildTheme(Brightness brightness) {
  final tokens = brightness == Brightness.dark
      ? AppTokens.dark
      : AppTokens.light;

  return ThemeData(
    brightness: brightness,
    colorScheme: ColorScheme(
      brightness: brightness,
      primary: tokens.primary,
      onPrimary: tokens.onPrimary,
      secondary: tokens.accent,
      onSecondary: const Color(0xFFFFFFFF),
      error: tokens.error,
      onError: const Color(0xFFFFFFFF),
      surface: tokens.canvas,
      onSurface: tokens.ink,
      outline: tokens.hairline,
    ),
    scaffoldBackgroundColor: tokens.canvas,
    dividerTheme: DividerThemeData(
      color: tokens.hairline,
      thickness: 1,
      space: 1,
    ),
    iconTheme: IconThemeData(color: tokens.body, size: 20),
    extensions: [tokens],
    textTheme: _textTheme(tokens),
  );
}

TextTheme _textTheme(AppTokens tokens) {
  return TextTheme(
    displaySmall: TextStyle(
      color: tokens.ink,
      fontSize: 28,
      fontWeight: FontWeight.w600,
      height: 1.2,
      letterSpacing: -0.5,
    ),
    titleLarge: TextStyle(
      color: tokens.ink,
      fontSize: 22,
      fontWeight: FontWeight.w600,
      height: 1.3,
      letterSpacing: -0.3,
    ),
    titleMedium: TextStyle(
      color: tokens.ink,
      fontSize: 18,
      fontWeight: FontWeight.w600,
      height: 1.4,
    ),
    titleSmall: TextStyle(
      color: tokens.ink,
      fontSize: 16,
      fontWeight: FontWeight.w600,
      height: 1.4,
    ),
    bodyLarge: TextStyle(
      color: tokens.body,
      fontSize: 16,
      fontWeight: FontWeight.w400,
      height: 1.5,
    ),
    bodyMedium: TextStyle(
      color: tokens.body,
      fontSize: 14,
      fontWeight: FontWeight.w400,
      height: 1.5,
    ),
    bodySmall: TextStyle(
      color: tokens.muted,
      fontSize: 13,
      fontWeight: FontWeight.w500,
      height: 1.4,
    ),
    labelLarge: TextStyle(
      color: tokens.ink,
      fontSize: 14,
      fontWeight: FontWeight.w600,
      height: 1,
    ),
    labelMedium: TextStyle(
      color: tokens.ink,
      fontSize: 14,
      fontWeight: FontWeight.w500,
      height: 1.4,
    ),
    labelSmall: TextStyle(
      color: tokens.muted,
      fontSize: 11,
      fontWeight: FontWeight.w500,
      height: 1.4,
    ),
  );
}
