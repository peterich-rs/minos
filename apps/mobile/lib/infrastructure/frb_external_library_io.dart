import 'package:flutter/foundation.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';

ExternalLibrary? resolveFrbExternalLibrary() {
  if (defaultTargetPlatform != TargetPlatform.iOS) {
    return null;
  }
  return ExternalLibrary.process(
    iKnowHowToUseIt: true,
    debugInfo: ' (libminos_ffi_frb.a is linked into Runner)',
  );
}
