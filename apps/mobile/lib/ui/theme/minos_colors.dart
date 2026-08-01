import 'package:flutter/material.dart';

/// Semantic color tokens for the Minos mobile design system.
///
/// Source of truth for golden-path surfaces. Prefer these over one-off
/// `Color(0x…)` literals and over shadcn/Material brand defaults.
@immutable
class MinosColors {
  const MinosColors({
    required this.brightness,
    required this.canvas,
    required this.surface,
    required this.surfaceElevated,
    required this.surfaceMuted,
    required this.border,
    required this.borderSubtle,
    required this.textPrimary,
    required this.textSecondary,
    required this.textTertiary,
    required this.textOnAccent,
    required this.accent,
    required this.accentMuted,
    required this.accentSoft,
    required this.success,
    required this.successSoft,
    required this.warning,
    required this.warningSoft,
    required this.danger,
    required this.dangerSoft,
    required this.userBubble,
    required this.userBubbleForeground,
    required this.assistantBubble,
    required this.navBar,
    required this.navSelected,
    required this.navUnselected,
    required this.scrim,
  });

  final Brightness brightness;

  final Color canvas;
  final Color surface;
  final Color surfaceElevated;
  final Color surfaceMuted;
  final Color border;
  final Color borderSubtle;

  final Color textPrimary;
  final Color textSecondary;
  final Color textTertiary;
  final Color textOnAccent;

  final Color accent;
  final Color accentMuted;
  final Color accentSoft;

  final Color success;
  final Color successSoft;
  final Color warning;
  final Color warningSoft;
  final Color danger;
  final Color dangerSoft;

  final Color userBubble;
  final Color userBubbleForeground;
  final Color assistantBubble;

  final Color navBar;
  final Color navSelected;
  final Color navUnselected;

  final Color scrim;

  bool get isDark => brightness == Brightness.dark;

  static const MinosColors light = MinosColors(
    brightness: Brightness.light,
    canvas: Color(0xFFF2F2F7),
    surface: Color(0xFFFFFFFF),
    surfaceElevated: Color(0xFFFFFFFF),
    surfaceMuted: Color(0xFFF2F2F7),
    border: Color(0xFFD1D1D6),
    borderSubtle: Color(0xFFE5E5EA),
    textPrimary: Color(0xFF1C1C1E),
    textSecondary: Color(0xFF636366),
    textTertiary: Color(0xFF8E8E93),
    textOnAccent: Color(0xFFFFFFFF),
    accent: Color(0xFF0A84FF),
    accentMuted: Color(0xFF007AFF),
    accentSoft: Color(0xFFE8F2FF),
    success: Color(0xFF34C759),
    successSoft: Color(0xFFE8F8ED),
    warning: Color(0xFFFF9F0A),
    warningSoft: Color(0xFFFFF4E0),
    danger: Color(0xFFFF3B30),
    dangerSoft: Color(0xFFFFEBEA),
    userBubble: Color(0xFF0A84FF),
    userBubbleForeground: Color(0xFFFFFFFF),
    assistantBubble: Color(0xFFFFFFFF),
    navBar: Color(0xF2F9F9F9),
    navSelected: Color(0xFF1C1C1E),
    navUnselected: Color(0xFF8E8E93),
    scrim: Color(0x66000000),
  );

  static const MinosColors dark = MinosColors(
    brightness: Brightness.dark,
    canvas: Color(0xFF000000),
    surface: Color(0xFF1C1C1E),
    surfaceElevated: Color(0xFF2C2C2E),
    surfaceMuted: Color(0xFF2C2C2E),
    border: Color(0xFF3A3A3C),
    borderSubtle: Color(0xFF2C2C2E),
    textPrimary: Color(0xFFF5F5F7),
    textSecondary: Color(0xFFAEAEB2),
    textTertiary: Color(0xFF8E8E93),
    textOnAccent: Color(0xFFFFFFFF),
    accent: Color(0xFF0A84FF),
    accentMuted: Color(0xFF409CFF),
    accentSoft: Color(0xFF0A2540),
    success: Color(0xFF30D158),
    successSoft: Color(0xFF0F2A18),
    warning: Color(0xFFFFD60A),
    warningSoft: Color(0xFF2A2208),
    danger: Color(0xFFFF453A),
    dangerSoft: Color(0xFF3A1210),
    userBubble: Color(0xFF0A84FF),
    userBubbleForeground: Color(0xFFFFFFFF),
    assistantBubble: Color(0xFF1C1C1E),
    navBar: Color(0xF21C1C1E),
    navSelected: Color(0xFFF5F5F7),
    navUnselected: Color(0xFF8E8E93),
    scrim: Color(0x99000000),
  );
}
