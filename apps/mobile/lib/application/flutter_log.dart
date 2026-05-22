import 'dart:developer' as developer;

import 'package:minos/src/rust/api/minos.dart';

const String _flutterLogPrefix = 'minos_mobile::flutter';

void logFlutterDebug(String target, String message) {
  _logFlutter(level: LogLevel.debug, target: target, message: message);
}

void logFlutterInfo(String target, String message) {
  _logFlutter(level: LogLevel.info, target: target, message: message);
}

void logFlutterWarn(
  String target,
  String message, {
  Object? error,
  StackTrace? stackTrace,
}) {
  _logFlutter(
    level: LogLevel.warn,
    target: target,
    message: message,
    error: error,
    stackTrace: stackTrace,
  );
}

void logFlutterError(
  String target,
  String message, {
  Object? error,
  StackTrace? stackTrace,
}) {
  _logFlutter(
    level: LogLevel.error,
    target: target,
    message: message,
    error: error,
    stackTrace: stackTrace,
  );
}

String flutterLogTarget(String target) =>
    '$_flutterLogPrefix::${target.trim().isEmpty ? 'app' : target.trim()}';

void _logFlutter({
  required LogLevel level,
  required String target,
  required String message,
  Object? error,
  StackTrace? stackTrace,
}) {
  final fullTarget = flutterLogTarget(target);
  final payload = <String>[
    message.trim().isEmpty ? '(empty message)' : message.trim(),
    if (error != null) 'error=${error.toString().trim()}',
    if (stackTrace != null) 'stack=$stackTrace',
  ].join('\n');

  try {
    emitLog(level: level, target: fullTarget, message: payload);
  } catch (emitError, emitStackTrace) {
    developer.log(
      payload,
      name: fullTarget,
      level: _developerLevel(level),
      error: error ?? emitError,
      stackTrace: stackTrace ?? emitStackTrace,
    );
  }
}

int _developerLevel(LogLevel level) {
  return switch (level) {
    LogLevel.trace => 300,
    LogLevel.debug => 500,
    LogLevel.info => 800,
    LogLevel.warn => 900,
    LogLevel.error => 1000,
  };
}
