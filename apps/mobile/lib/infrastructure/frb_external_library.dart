import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';

import 'frb_external_library_io.dart'
    if (dart.library.js_interop) 'frb_external_library_web.dart'
    as impl;

ExternalLibrary? resolveFrbExternalLibrary() =>
    impl.resolveFrbExternalLibrary();
