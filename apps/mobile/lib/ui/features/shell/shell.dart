/// Feature: App Shell
///
/// The root navigation shell with tab bar (Messages / Members / Profile).
/// Owns the tab switching logic and the root route decision.
///
/// View Models:
///   - [RootRoute] / [decideRootRoute] (application/root_route_decision.dart)
///
/// Views:
///   - [AppShellPage]
///   - [MinosApp] (presentation/app.dart)
library;

export 'package:minos/application/root_route_decision.dart';
export 'package:minos/presentation/app.dart';
export 'package:minos/presentation/pages/app_shell_page.dart';
