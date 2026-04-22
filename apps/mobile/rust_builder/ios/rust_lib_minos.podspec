#
# To learn more about a Podspec see http://guides.cocoapods.org/syntax/podspec.html.
# Run `pod lib lint rust_lib_minos.podspec` to validate before publishing.
#
# Minos note: the rust crate lives in the workspace root, not under
# `apps/mobile/rust/`. Path arg below is relative to $PODS_TARGET_SRCROOT
# (which is `apps/mobile/rust_builder/ios`). Library name `minos_ffi_frb`
# matches cargo's default `[lib]` name derived from the crate's package name
# (dashes → underscores).
#
Pod::Spec.new do |s|
  s.name             = 'rust_lib_minos'
  s.version          = '0.0.1'
  s.summary          = 'Cargokit builder plugin for minos-ffi-frb.'
  s.description      = <<-DESC
Cargokit builder plugin that compiles and links `crates/minos-ffi-frb` into
the iOS host binary so the frb-generated Dart bindings can resolve their
Rust symbols.
                       DESC
  s.homepage         = 'http://example.com'
  s.license          = { :file => '../LICENSE' }
  s.author           = { 'Minos' => 'noreply@example.com' }

  # This will ensure the source files in Classes/ are included in the native
  # builds of apps using this FFI plugin. Podspec does not support relative
  # paths, so Classes contains a forwarder C file that relatively imports
  # `../src/*` so that the C sources can be shared among all target platforms.
  s.source           = { :path => '.' }
  s.source_files = 'Classes/**/*'
  s.dependency 'Flutter'
  s.platform = :ios, '16.0'

  # Flutter.framework does not contain a i386 slice.
  s.pod_target_xcconfig = { 'DEFINES_MODULE' => 'YES', 'EXCLUDED_ARCHS[sdk=iphonesimulator*]' => 'i386' }
  s.swift_version = '5.0'

  s.script_phase = {
    :name => 'Build Rust library',
    # First argument: path to the cargo manifest dir, relative to the pod.
    #   From $PODS_TARGET_SRCROOT (= apps/mobile/rust_builder/ios) up four levels
    #   to the repo root, then into crates/minos-ffi-frb.
    # Second argument: legacy cargokit positional (library name hint; the
    #   actual library filename is derived from the Cargo.toml package name).
    :script => 'sh "$PODS_TARGET_SRCROOT/../cargokit/build_pod.sh" ../../../../crates/minos-ffi-frb minos_ffi_frb',
    :execution_position => :before_compile,
    :input_files => ['${BUILT_PRODUCTS_DIR}/cargokit_phony'],
    # Let XCode know that the static library referenced in -force_load below is
    # created by this build step.
    :output_files => ["${BUILT_PRODUCTS_DIR}/libminos_ffi_frb.a"],
  }
  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
    # Flutter.framework does not contain a i386 slice.
    'EXCLUDED_ARCHS[sdk=iphonesimulator*]' => 'i386',
    'OTHER_LDFLAGS' => '-force_load ${BUILT_PRODUCTS_DIR}/libminos_ffi_frb.a',
  }
end
