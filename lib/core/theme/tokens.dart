import 'package:flutter/material.dart';

/// MailSlate design tokens (DESIGN.md §3) exposed as a [ThemeExtension].
///
/// Widgets read tokens via `context.tokens` (or
/// `Theme.of(context).extension<AppTokens>()!`) so swapping light/dark is
/// a single theme change with no widget branches.
@immutable
class AppTokens extends ThemeExtension<AppTokens> {
  const AppTokens({
    required this.canvas,
    required this.surfaceSoft,
    required this.surfaceCard,
    required this.surfaceStrong,
    required this.hairline,
    required this.hairlineSoft,
    required this.ink,
    required this.body,
    required this.muted,
    required this.mutedSoft,
    required this.onPrimary,
    required this.primary,
    required this.primaryActive,
    required this.primaryDisabled,
    required this.accent,
    required this.accentTint,
    required this.star,
    required this.success,
    required this.warning,
    required this.error,
    required this.badgeOrange,
    required this.badgePink,
    required this.badgeViolet,
    required this.badgeEmerald,
  });

  /// Page background.
  final Color canvas;

  /// Sidebar and toolbar background.
  final Color surfaceSoft;

  /// Cards and raised containers.
  final Color surfaceCard;

  /// Strongest neutral surface (hover chrome).
  final Color surfaceStrong;

  /// Visible separators.
  final Color hairline;

  /// Subtle separators.
  final Color hairlineSoft;

  /// Primary text.
  final Color ink;

  /// Secondary text.
  final Color body;

  /// Tertiary text and metadata.
  final Color muted;

  /// Faint meta text (timestamps disabled states).
  final Color mutedSoft;

  /// Text/icon color on [primary].
  final Color onPrimary;

  /// Primary action background.
  final Color primary;

  /// Primary action hover/pressed background.
  final Color primaryActive;

  /// Disabled fill for primary actions.
  final Color primaryDisabled;

  /// Links, selection, focus.
  final Color accent;

  /// 12–20% accent wash for selected rows.
  final Color accentTint;

  /// Starred marker.
  final Color star;

  /// Success status.
  final Color success;

  /// Warning status.
  final Color warning;

  /// Error status.
  final Color error;

  /// Sender identity pastels (DESIGN §3.3).
  final Color badgeOrange;
  final Color badgePink;
  final Color badgeViolet;
  final Color badgeEmerald;

  /// Light theme (DESIGN.md §3.1).
  static const AppTokens light = AppTokens(
    canvas: Color(0xFFFFFFFF),
    surfaceSoft: Color(0xFFF8F9FA),
    surfaceCard: Color(0xFFF5F5F5),
    surfaceStrong: Color(0xFFE5E7EB),
    hairline: Color(0xFFE5E7EB),
    hairlineSoft: Color(0xFFF3F4F6),
    ink: Color(0xFF111111),
    body: Color(0xFF374151),
    muted: Color(0xFF6B7280),
    mutedSoft: Color(0xFF898989),
    onPrimary: Color(0xFFFFFFFF),
    primary: Color(0xFF111111),
    primaryActive: Color(0xFF242424),
    primaryDisabled: Color(0xFFE5E7EB),
    accent: Color(0xFF3B82F6),
    accentTint: Color(0x1F3B82F6),
    star: Color(0xFFFB923C),
    success: Color(0xFF10B981),
    warning: Color(0xFFF59E0B),
    error: Color(0xFFEF4444),
    badgeOrange: Color(0xFFFB923C),
    badgePink: Color(0xFFEC4899),
    badgeViolet: Color(0xFF8B5CF6),
    badgeEmerald: Color(0xFF34D399),
  );

  /// Dark theme (DESIGN.md §3.2).
  static const AppTokens dark = AppTokens(
    canvas: Color(0xFF101010),
    surfaceSoft: Color(0xFF151515),
    surfaceCard: Color(0xFF1A1A1A),
    surfaceStrong: Color(0xFF262626),
    hairline: Color(0x1AFFFFFF),
    hairlineSoft: Color(0x0FFFFFFF),
    ink: Color(0xFFFAFAFA),
    body: Color(0xFFD4D4D8),
    muted: Color(0xFFA1A1AA),
    mutedSoft: Color(0xFF71717A),
    onPrimary: Color(0xFF111111),
    primary: Color(0xFFFAFAFA),
    primaryActive: Color(0xFFE5E7EB),
    primaryDisabled: Color(0xFF262626),
    accent: Color(0xFF3B82F6),
    accentTint: Color(0x333B82F6),
    star: Color(0xFFFB923C),
    success: Color(0xFF10B981),
    warning: Color(0xFFF59E0B),
    error: Color(0xFFEF4444),
    badgeOrange: Color(0xFFFB923C),
    badgePink: Color(0xFFEC4899),
    badgeViolet: Color(0xFF8B5CF6),
    badgeEmerald: Color(0xFF34D399),
  );

  @override
  AppTokens copyWith({
    Color? canvas,
    Color? surfaceSoft,
    Color? surfaceCard,
    Color? surfaceStrong,
    Color? hairline,
    Color? hairlineSoft,
    Color? ink,
    Color? body,
    Color? muted,
    Color? mutedSoft,
    Color? onPrimary,
    Color? primary,
    Color? primaryActive,
    Color? primaryDisabled,
    Color? accent,
    Color? accentTint,
    Color? star,
    Color? success,
    Color? warning,
    Color? error,
    Color? badgeOrange,
    Color? badgePink,
    Color? badgeViolet,
    Color? badgeEmerald,
  }) {
    return AppTokens(
      canvas: canvas ?? this.canvas,
      surfaceSoft: surfaceSoft ?? this.surfaceSoft,
      surfaceCard: surfaceCard ?? this.surfaceCard,
      surfaceStrong: surfaceStrong ?? this.surfaceStrong,
      hairline: hairline ?? this.hairline,
      hairlineSoft: hairlineSoft ?? this.hairlineSoft,
      ink: ink ?? this.ink,
      body: body ?? this.body,
      muted: muted ?? this.muted,
      mutedSoft: mutedSoft ?? this.mutedSoft,
      onPrimary: onPrimary ?? this.onPrimary,
      primary: primary ?? this.primary,
      primaryActive: primaryActive ?? this.primaryActive,
      primaryDisabled: primaryDisabled ?? this.primaryDisabled,
      accent: accent ?? this.accent,
      accentTint: accentTint ?? this.accentTint,
      star: star ?? this.star,
      success: success ?? this.success,
      warning: warning ?? this.warning,
      error: error ?? this.error,
      badgeOrange: badgeOrange ?? this.badgeOrange,
      badgePink: badgePink ?? this.badgePink,
      badgeViolet: badgeViolet ?? this.badgeViolet,
      badgeEmerald: badgeEmerald ?? this.badgeEmerald,
    );
  }

  @override
  AppTokens lerp(ThemeExtension<AppTokens>? other, double t) {
    if (other is! AppTokens) {
      return this;
    }
    return AppTokens(
      canvas: Color.lerp(canvas, other.canvas, t)!,
      surfaceSoft: Color.lerp(surfaceSoft, other.surfaceSoft, t)!,
      surfaceCard: Color.lerp(surfaceCard, other.surfaceCard, t)!,
      surfaceStrong: Color.lerp(surfaceStrong, other.surfaceStrong, t)!,
      hairline: Color.lerp(hairline, other.hairline, t)!,
      hairlineSoft: Color.lerp(hairlineSoft, other.hairlineSoft, t)!,
      ink: Color.lerp(ink, other.ink, t)!,
      body: Color.lerp(body, other.body, t)!,
      muted: Color.lerp(muted, other.muted, t)!,
      mutedSoft: Color.lerp(mutedSoft, other.mutedSoft, t)!,
      onPrimary: Color.lerp(onPrimary, other.onPrimary, t)!,
      primary: Color.lerp(primary, other.primary, t)!,
      primaryActive: Color.lerp(primaryActive, other.primaryActive, t)!,
      primaryDisabled: Color.lerp(primaryDisabled, other.primaryDisabled, t)!,
      accent: Color.lerp(accent, other.accent, t)!,
      accentTint: Color.lerp(accentTint, other.accentTint, t)!,
      star: Color.lerp(star, other.star, t)!,
      success: Color.lerp(success, other.success, t)!,
      warning: Color.lerp(warning, other.warning, t)!,
      error: Color.lerp(error, other.error, t)!,
      badgeOrange: Color.lerp(badgeOrange, other.badgeOrange, t)!,
      badgePink: Color.lerp(badgePink, other.badgePink, t)!,
      badgeViolet: Color.lerp(badgeViolet, other.badgeViolet, t)!,
      badgeEmerald: Color.lerp(badgeEmerald, other.badgeEmerald, t)!,
    );
  }
}

/// Shorthand for `Theme.of(context).extension<AppTokens>()!`.
extension AppTokensContext on BuildContext {
  AppTokens get tokens => Theme.of(this).extension<AppTokens>()!;
}
