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
///   - [MinosApp]
library;

export 'package:minos/application/root_route_decision.dart';
export 'package:minos/ui/features/shell/router.dart';
export 'package:minos/ui/features/shell/router_provider.dart';
export 'package:minos/ui/features/shell/views/app.dart';
export 'package:minos/ui/features/shell/views/app_shell_page.dart';
