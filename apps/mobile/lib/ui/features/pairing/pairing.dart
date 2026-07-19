/// Feature: Device Pairing
///
/// QR-code scanning and device pairing flow. Manages camera permissions
/// and the pairing submission lifecycle.
///
/// View Models:
///   - [PairingController] (application/minos_providers.dart)
///   - [CameraPermission] (application/minos_providers.dart)
///
/// Views:
///   - [PairingPage]
///   - [PermissionDeniedPage]
library;

export 'package:minos/application/minos_providers.dart';
export 'package:minos/ui/features/pairing/views/pairing_page.dart';
export 'package:minos/ui/features/pairing/views/permission_denied_page.dart';
export 'package:minos/ui/features/pairing/widgets/qr_scanner_view.dart';
