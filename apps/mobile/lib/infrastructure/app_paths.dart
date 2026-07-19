import 'dart:io';

import 'package:path_provider/path_provider.dart';

Future<Directory> minosAppDirectory() async {
  final docs = await getApplicationDocumentsDirectory();
  final dir = Directory('${docs.path}/Minos');
  await dir.create(recursive: true);
  return dir;
}

/// Resolve and ensure the Minos log directory inside the app's Documents
/// sandbox. Returns the absolute path suitable for passing to the Rust
/// `init_logging` entry point.
Future<String> logDirectory() async {
  final root = await minosAppDirectory();
  final dir = Directory('${root.path}/Logs');
  await dir.create(recursive: true);
  return dir.path;
}

Future<String> agentProfilesFilePath() async {
  final root = await minosAppDirectory();
  return '${root.path}/agent_profiles.json';
}
