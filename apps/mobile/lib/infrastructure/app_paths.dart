import 'dart:io';

import 'package:path_provider/path_provider.dart';

/// Resolve and ensure the Minos log directory inside the app's Documents
/// sandbox. Returns the absolute path suitable for passing to the Rust
/// `init_logging` entry point.
Future<String> logDirectory() async {
  final docs = await getApplicationDocumentsDirectory();
  final dir = Directory('${docs.path}/Minos/Logs');
  await dir.create(recursive: true);
  return dir.path;
}
