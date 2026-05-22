/// Feature: Authentication
///
/// Login / register surface. Owns the auth form, error banners, and the
/// mode toggle between login and register.
///
/// View Models:
///   - [AuthController] (application/auth_provider.dart)
///
/// Views:
///   - [LoginPage]
///   - [AuthForm], [AuthErrorBanner]
library;

export 'package:minos/application/auth_provider.dart';
export 'package:minos/ui/features/auth/views/login_page.dart';
export 'package:minos/ui/features/auth/widgets/auth_error_banner.dart';
export 'package:minos/ui/features/auth/widgets/auth_form.dart';
