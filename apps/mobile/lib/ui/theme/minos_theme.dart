import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:minos/ui/theme/minos_colors.dart';
import 'package:minos/ui/theme/minos_radii.dart';
import 'package:minos/ui/theme/minos_typography.dart';

/// Builds [ThemeData] from [MinosColors] and exposes tokens via
/// [MinosThemeExtension].
abstract final class MinosTheme {
  static ThemeData light() => _build(MinosColors.light);

  static ThemeData dark() => _build(MinosColors.dark);

  static ThemeData _build(MinosColors colors) {
    final textTheme = MinosTypography.textTheme(
      colors.textPrimary,
      colors.textSecondary,
    );
    final base = colors.isDark ? ThemeData.dark() : ThemeData.light();

    return base.copyWith(
      brightness: colors.brightness,
      scaffoldBackgroundColor: colors.canvas,
      colorScheme: ColorScheme(
        brightness: colors.brightness,
        primary: colors.accent,
        onPrimary: colors.textOnAccent,
        secondary: colors.accentMuted,
        onSecondary: colors.textOnAccent,
        error: colors.danger,
        onError: colors.textOnAccent,
        surface: colors.surface,
        onSurface: colors.textPrimary,
        onSurfaceVariant: colors.textSecondary,
        outline: colors.border,
        outlineVariant: colors.borderSubtle,
        surfaceContainerLowest: colors.canvas,
        surfaceContainerLow: colors.surfaceMuted,
        surfaceContainer: colors.surface,
        surfaceContainerHigh: colors.surfaceElevated,
        surfaceContainerHighest: colors.surfaceMuted,
      ),
      textTheme: textTheme,
      primaryTextTheme: textTheme,
      appBarTheme: AppBarTheme(
        elevation: 0,
        scrolledUnderElevation: 0,
        centerTitle: false,
        backgroundColor: colors.canvas,
        foregroundColor: colors.textPrimary,
        surfaceTintColor: Colors.transparent,
        systemOverlayStyle: colors.isDark
            ? SystemUiOverlayStyle.light
            : SystemUiOverlayStyle.dark,
        titleTextStyle: textTheme.titleLarge,
      ),
      cardTheme: CardThemeData(
        color: colors.surface,
        elevation: 0,
        margin: EdgeInsets.zero,
        shape: const RoundedRectangleBorder(borderRadius: MinosRadii.mdAll),
      ),
      dividerTheme: DividerThemeData(
        color: colors.borderSubtle,
        thickness: 0.5,
        space: 0.5,
      ),
      inputDecorationTheme: InputDecorationTheme(
        filled: true,
        fillColor: colors.surfaceMuted,
        contentPadding: const EdgeInsets.symmetric(
          horizontal: 14,
          vertical: 14,
        ),
        border: const OutlineInputBorder(
          borderRadius: MinosRadii.smAll,
          borderSide: BorderSide.none,
        ),
        enabledBorder: const OutlineInputBorder(
          borderRadius: MinosRadii.smAll,
          borderSide: BorderSide.none,
        ),
        focusedBorder: OutlineInputBorder(
          borderRadius: MinosRadii.smAll,
          borderSide: BorderSide(color: colors.accent, width: 1.5),
        ),
        errorBorder: OutlineInputBorder(
          borderRadius: MinosRadii.smAll,
          borderSide: BorderSide(color: colors.danger),
        ),
        focusedErrorBorder: OutlineInputBorder(
          borderRadius: MinosRadii.smAll,
          borderSide: BorderSide(color: colors.danger, width: 1.5),
        ),
        hintStyle: textTheme.bodyMedium?.copyWith(color: colors.textTertiary),
        errorStyle: textTheme.bodySmall?.copyWith(color: colors.danger),
      ),
      filledButtonTheme: FilledButtonThemeData(
        style: FilledButton.styleFrom(
          backgroundColor: colors.accent,
          foregroundColor: colors.textOnAccent,
          disabledBackgroundColor: colors.accent.withValues(alpha: 0.35),
          disabledForegroundColor: colors.textOnAccent.withValues(alpha: 0.7),
          elevation: 0,
          minimumSize: const Size.fromHeight(48),
          shape: const RoundedRectangleBorder(borderRadius: MinosRadii.smAll),
          textStyle: textTheme.labelLarge,
        ),
      ),
      outlinedButtonTheme: OutlinedButtonThemeData(
        style: OutlinedButton.styleFrom(
          foregroundColor: colors.textPrimary,
          minimumSize: const Size.fromHeight(48),
          side: BorderSide(color: colors.border),
          shape: const RoundedRectangleBorder(borderRadius: MinosRadii.smAll),
          textStyle: textTheme.labelLarge,
        ),
      ),
      textButtonTheme: TextButtonThemeData(
        style: TextButton.styleFrom(
          foregroundColor: colors.accent,
          textStyle: textTheme.labelLarge,
        ),
      ),
      iconButtonTheme: IconButtonThemeData(
        style: IconButton.styleFrom(
          foregroundColor: colors.textPrimary,
          disabledForegroundColor: colors.textTertiary,
        ),
      ),
      navigationBarTheme: NavigationBarThemeData(
        height: 64,
        backgroundColor: colors.navBar,
        indicatorColor: colors.accentSoft,
        surfaceTintColor: Colors.transparent,
        elevation: 0,
        labelTextStyle: WidgetStateProperty.resolveWith((states) {
          final selected = states.contains(WidgetState.selected);
          return textTheme.labelSmall?.copyWith(
            color: selected ? colors.navSelected : colors.navUnselected,
            fontWeight: selected ? FontWeight.w700 : FontWeight.w500,
          );
        }),
        iconTheme: WidgetStateProperty.resolveWith((states) {
          final selected = states.contains(WidgetState.selected);
          return IconThemeData(
            size: 22,
            color: selected ? colors.navSelected : colors.navUnselected,
          );
        }),
      ),
      snackBarTheme: SnackBarThemeData(
        backgroundColor: colors.surfaceElevated,
        contentTextStyle: textTheme.bodyMedium,
        behavior: SnackBarBehavior.floating,
        shape: const RoundedRectangleBorder(borderRadius: MinosRadii.smAll),
      ),
      bottomSheetTheme: BottomSheetThemeData(
        backgroundColor: colors.surface,
        surfaceTintColor: Colors.transparent,
        shape: const RoundedRectangleBorder(borderRadius: MinosRadii.sheetTop),
        showDragHandle: false,
      ),
      progressIndicatorTheme: ProgressIndicatorThemeData(color: colors.accent),
      extensions: <ThemeExtension<dynamic>>[MinosThemeExtension(colors)],
    );
  }
}

/// Theme extension so widgets can read [MinosColors] without
/// re-deriving from [ColorScheme].
@immutable
class MinosThemeExtension extends ThemeExtension<MinosThemeExtension> {
  const MinosThemeExtension(this.colors);

  final MinosColors colors;

  @override
  MinosThemeExtension copyWith({MinosColors? colors}) {
    return MinosThemeExtension(colors ?? this.colors);
  }

  @override
  MinosThemeExtension lerp(
    ThemeExtension<MinosThemeExtension>? other,
    double t,
  ) {
    if (other is! MinosThemeExtension) return this;
    // Discrete tokens — snap at midpoint rather than interpolating every color.
    return t < 0.5 ? this : other;
  }
}

/// Convenience accessors for Minos design tokens.
extension MinosThemeContext on BuildContext {
  MinosColors get minosColors {
    final ext = Theme.of(this).extension<MinosThemeExtension>();
    if (ext != null) return ext.colors;
    return Theme.of(this).brightness == Brightness.dark
        ? MinosColors.dark
        : MinosColors.light;
  }
}
