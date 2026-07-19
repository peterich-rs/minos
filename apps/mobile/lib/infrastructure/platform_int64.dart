import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';

PlatformInt64 platformInt64FromInt(int value) => PlatformInt64Util.from(value);

int platformInt64ToInt(PlatformInt64 value) => value.toInt();

int? platformInt64ToNullableInt(PlatformInt64? value) => value?.toInt();
