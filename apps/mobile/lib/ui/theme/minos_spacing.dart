/// Spacing scale used across golden-path mobile surfaces.
///
/// Prefer these constants over magic numbers so density stays consistent.
abstract final class MinosSpacing {
  static const double xxs = 2;
  static const double xs = 4;
  static const double sm = 8;
  static const double md = 12;
  static const double lg = 16;
  static const double xl = 20;
  static const double xxl = 24;
  static const double xxxl = 32;
  static const double huge = 48;

  /// Horizontal page inset for list / form content.
  static const double pageX = 16;

  /// Top inset under large titles.
  static const double pageTop = 8;

  /// Bottom padding above the tab bar / home indicator.
  static const double pageBottom = 28;
}
