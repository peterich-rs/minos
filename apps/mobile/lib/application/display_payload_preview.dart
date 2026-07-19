import 'package:minos/src/rust/api/minos.dart';

extension DisplayPayloadPreview on DisplayPayload {
  String renderPreview() {
    return switch (this) {
      DisplayPayload_Inline(:final text) => text,
      DisplayPayload_StreamingWindow(:final head, :final receivedBytes) =>
        _appendMarker(head, 'streaming ${_formatBytes(receivedBytes)}'),
      DisplayPayload_WindowedFinal(
        :final head,
        :final tail,
        :final omittedBytes,
      ) =>
        omittedBytes == BigInt.zero
            ? '$head$tail'
            : _joinWindow(head, tail, omittedBytes),
    };
  }

  ArtifactRef? get artifactRef {
    return switch (this) {
      DisplayPayload_Inline() => null,
      DisplayPayload_StreamingWindow(:final artifact) => artifact,
      DisplayPayload_WindowedFinal(:final artifact) => artifact,
    };
  }
}

String _joinWindow(String head, String tail, BigInt omittedBytes) {
  final marker = '[... ${_formatBytes(omittedBytes)} omitted ...]';
  if (head.isEmpty) return '$marker\n$tail';
  if (tail.isEmpty) return '$head\n$marker';
  return '$head\n$marker\n$tail';
}

String _appendMarker(String text, String marker) {
  if (text.isEmpty) return '[$marker]';
  return '$text\n[$marker]';
}

String _formatBytes(BigInt value) {
  final bytes = value.toDouble();
  if (bytes < 1024) return '${value.toString()} B';
  final kib = bytes / 1024;
  if (kib < 1024) return '${kib.toStringAsFixed(1)} KiB';
  final mib = kib / 1024;
  return '${mib.toStringAsFixed(1)} MiB';
}
