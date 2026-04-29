import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/application/request_trace_records_provider.dart';
import 'package:minos/src/rust/api/minos.dart';

enum _TraceTransportFilter { all, http, rpc }

extension on _TraceTransportFilter {
  String get label {
    return switch (this) {
      _TraceTransportFilter.all => '全部',
      _TraceTransportFilter.http => 'HTTP',
      _TraceTransportFilter.rpc => 'RPC',
    };
  }

  bool includes(RequestTraceTransport transport) {
    return switch (this) {
      _TraceTransportFilter.all => true,
      _TraceTransportFilter.http => transport == RequestTraceTransport.http,
      _TraceTransportFilter.rpc => transport == RequestTraceTransport.rpc,
    };
  }
}

enum _TraceStatusFilter { all, pending, success, failure }

extension on _TraceStatusFilter {
  String get label {
    return switch (this) {
      _TraceStatusFilter.all => '全部状态',
      _TraceStatusFilter.pending => '进行中',
      _TraceStatusFilter.success => '成功',
      _TraceStatusFilter.failure => '失败',
    };
  }

  bool includes(RequestTraceStatus status) {
    return switch (this) {
      _TraceStatusFilter.all => true,
      _TraceStatusFilter.pending => status == RequestTraceStatus.pending,
      _TraceStatusFilter.success => status == RequestTraceStatus.success,
      _TraceStatusFilter.failure => status == RequestTraceStatus.failure,
    };
  }
}

class RequestTracePanel extends ConsumerStatefulWidget {
  const RequestTracePanel({
    super.key,
    this.height = 240,
    this.showControls = false,
  });

  final double height;
  final bool showControls;

  @override
  ConsumerState<RequestTracePanel> createState() => _RequestTracePanelState();
}

class _RequestTracePanelState extends ConsumerState<RequestTracePanel> {
  _TraceTransportFilter _transportFilter = _TraceTransportFilter.all;
  _TraceStatusFilter _statusFilter = _TraceStatusFilter.all;

  @override
  Widget build(BuildContext context) {
    final traces = ref.watch(RequestTraceRecords.provider);
    final visible = traces
        .where((trace) => _transportFilter.includes(trace.transport))
        .where((trace) => _statusFilter.includes(trace.status))
        .toList(growable: false)
        .reversed
        .toList(growable: false);

    return SizedBox(
      height: widget.height,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: <Widget>[
          if (widget.showControls) ...<Widget>[
            Padding(
              padding: const EdgeInsets.fromLTRB(8, 8, 8, 4),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: <Widget>[
                  _FilterRow<_TraceTransportFilter>(
                    value: _transportFilter,
                    values: _TraceTransportFilter.values,
                    label: (filter) => filter.label,
                    onSelected: (value) {
                      setState(() => _transportFilter = value);
                    },
                  ),
                  const SizedBox(height: 8),
                  Row(
                    children: <Widget>[
                      Expanded(
                        child: _FilterRow<_TraceStatusFilter>(
                          value: _statusFilter,
                          values: _TraceStatusFilter.values,
                          label: (filter) => filter.label,
                          onSelected: (value) {
                            setState(() => _statusFilter = value);
                          },
                        ),
                      ),
                      const SizedBox(width: 8),
                      TextButton(
                        onPressed: () {
                          ref
                              .read(RequestTraceRecords.provider.notifier)
                              .clear();
                        },
                        child: const Text('清空'),
                      ),
                    ],
                  ),
                ],
              ),
            ),
            const Divider(height: 1),
          ],
          Expanded(
            child: visible.isEmpty
                ? Center(
                    child: Text(
                      traces.isEmpty ? '暂无请求记录' : '当前筛选下暂无请求',
                      style: TextStyle(
                        color: Theme.of(context).colorScheme.onSurfaceVariant,
                      ),
                    ),
                  )
                : ListView.separated(
                    padding: const EdgeInsets.symmetric(
                      horizontal: 12,
                      vertical: 8,
                    ),
                    itemCount: visible.length,
                    separatorBuilder: (_, _) => const SizedBox(height: 8),
                    itemBuilder: (_, index) =>
                        _TraceCard(trace: visible[index]),
                  ),
          ),
        ],
      ),
    );
  }
}

class _FilterRow<T> extends StatelessWidget {
  const _FilterRow({
    required this.value,
    required this.values,
    required this.label,
    required this.onSelected,
  });

  final T value;
  final List<T> values;
  final String Function(T value) label;
  final ValueChanged<T> onSelected;

  @override
  Widget build(BuildContext context) {
    return SingleChildScrollView(
      scrollDirection: Axis.horizontal,
      child: Row(
        children: values
            .map(
              (entry) => Padding(
                padding: const EdgeInsets.only(right: 8),
                child: ChoiceChip(
                  label: Text(label(entry)),
                  selected: value == entry,
                  onSelected: (_) => onSelected(entry),
                ),
              ),
            )
            .toList(growable: false),
      ),
    );
  }
}

class _TraceCard extends StatelessWidget {
  const _TraceCard({required this.trace});

  final RequestTraceRecord trace;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final summary =
        trace.errorDetail ??
        trace.responseSummary ??
        trace.requestSummary ??
        '—';

    return Card(
      elevation: 0,
      color: theme.colorScheme.surfaceContainerLow,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(18)),
      child: InkWell(
        borderRadius: BorderRadius.circular(18),
        onTap: () => _showDetails(context),
        child: Padding(
          padding: const EdgeInsets.all(14),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: <Widget>[
              Row(
                children: <Widget>[
                  _Tag(
                    label: trace.transport == RequestTraceTransport.http
                        ? 'HTTP'
                        : 'RPC',
                    background: trace.transport == RequestTraceTransport.http
                        ? const Color(0x1A1F6FEB)
                        : const Color(0x1A238636),
                    foreground: trace.transport == RequestTraceTransport.http
                        ? const Color(0xFF1F6FEB)
                        : const Color(0xFF238636),
                  ),
                  const SizedBox(width: 8),
                  _Tag(
                    label: _statusLabel(trace.status),
                    background: _statusBackground(trace.status),
                    foreground: _statusForeground(trace.status),
                  ),
                  const Spacer(),
                  Text(
                    _durationLabel(trace),
                    style: theme.textTheme.labelMedium?.copyWith(
                      color: theme.colorScheme.onSurfaceVariant,
                    ),
                  ),
                ],
              ),
              const SizedBox(height: 10),
              Text(
                '${trace.method} ${trace.target}',
                style: theme.textTheme.titleSmall?.copyWith(
                  fontWeight: FontWeight.w700,
                ),
              ),
              const SizedBox(height: 6),
              if (trace.threadId != null)
                Text(
                  'thread ${trace.threadId}',
                  style: theme.textTheme.bodySmall?.copyWith(
                    color: theme.colorScheme.onSurfaceVariant,
                  ),
                ),
              if (trace.threadId != null) const SizedBox(height: 6),
              Text(
                summary,
                maxLines: 2,
                overflow: TextOverflow.ellipsis,
                style: theme.textTheme.bodyMedium?.copyWith(
                  color: theme.colorScheme.onSurfaceVariant,
                  height: 1.35,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }

  Future<void> _showDetails(BuildContext context) {
    final theme = Theme.of(context);
    return showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      showDragHandle: true,
      builder: (context) {
        return SafeArea(
          child: Padding(
            padding: const EdgeInsets.fromLTRB(20, 8, 20, 24),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: <Widget>[
                Text(
                  '${trace.method} ${trace.target}',
                  style: theme.textTheme.titleLarge?.copyWith(
                    fontWeight: FontWeight.w800,
                  ),
                ),
                const SizedBox(height: 16),
                _DetailRow(
                  label: '传输',
                  value: trace.transport.name.toUpperCase(),
                ),
                _DetailRow(label: '状态', value: _statusLabel(trace.status)),
                _DetailRow(
                  label: '状态码',
                  value: trace.statusCode?.toString() ?? '—',
                ),
                _DetailRow(label: '耗时', value: _durationLabel(trace)),
                _DetailRow(
                  label: '开始',
                  value: _formatTimestamp(trace.startedAtMs.toInt()),
                ),
                _DetailRow(
                  label: '完成',
                  value: trace.completedAtMs == null
                      ? '—'
                      : _formatTimestamp(trace.completedAtMs!.toInt()),
                ),
                _DetailRow(label: 'thread', value: trace.threadId ?? '—'),
                _DetailBlock(label: '请求摘要', value: trace.requestSummary),
                _DetailBlock(label: '响应摘要', value: trace.responseSummary),
                _DetailBlock(label: '错误详情', value: trace.errorDetail),
              ],
            ),
          ),
        );
      },
    );
  }

  String _durationLabel(RequestTraceRecord trace) {
    if (trace.durationMs != null) {
      return '${trace.durationMs} ms';
    }
    if (trace.status == RequestTraceStatus.pending) {
      return '进行中';
    }
    return '—';
  }

  String _statusLabel(RequestTraceStatus status) {
    return switch (status) {
      RequestTraceStatus.pending => '进行中',
      RequestTraceStatus.success => '成功',
      RequestTraceStatus.failure => '失败',
    };
  }

  Color _statusBackground(RequestTraceStatus status) {
    return switch (status) {
      RequestTraceStatus.pending => const Color(0x1A1F6FEB),
      RequestTraceStatus.success => const Color(0x1A238636),
      RequestTraceStatus.failure => const Color(0x1AF85149),
    };
  }

  Color _statusForeground(RequestTraceStatus status) {
    return switch (status) {
      RequestTraceStatus.pending => const Color(0xFF1F6FEB),
      RequestTraceStatus.success => const Color(0xFF238636),
      RequestTraceStatus.failure => const Color(0xFFF85149),
    };
  }

  String _formatTimestamp(int tsMs) {
    final ts = DateTime.fromMillisecondsSinceEpoch(tsMs, isUtc: false);
    final hh = ts.hour.toString().padLeft(2, '0');
    final mm = ts.minute.toString().padLeft(2, '0');
    final ss = ts.second.toString().padLeft(2, '0');
    final ms = ts.millisecond.toString().padLeft(3, '0');
    return '$hh:$mm:$ss.$ms';
  }
}

class _Tag extends StatelessWidget {
  const _Tag({
    required this.label,
    required this.background,
    required this.foreground,
  });

  final String label;
  final Color background;
  final Color foreground;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
      decoration: BoxDecoration(
        color: background,
        borderRadius: BorderRadius.circular(999),
      ),
      child: Text(
        label,
        style: TextStyle(color: foreground, fontWeight: FontWeight.w700),
      ),
    );
  }
}

class _DetailRow extends StatelessWidget {
  const _DetailRow({required this.label, required this.value});

  final String label;
  final String value;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.only(bottom: 10),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: <Widget>[
          SizedBox(
            width: 72,
            child: Text(
              label,
              style: theme.textTheme.bodySmall?.copyWith(
                color: theme.colorScheme.onSurfaceVariant,
              ),
            ),
          ),
          Expanded(
            child: Text(
              value,
              style: theme.textTheme.bodyMedium?.copyWith(height: 1.35),
            ),
          ),
        ],
      ),
    );
  }
}

class _DetailBlock extends StatelessWidget {
  const _DetailBlock({required this.label, required this.value});

  final String label;
  final String? value;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.only(top: 6),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: <Widget>[
          Text(
            label,
            style: theme.textTheme.bodySmall?.copyWith(
              color: theme.colorScheme.onSurfaceVariant,
            ),
          ),
          const SizedBox(height: 4),
          Container(
            width: double.infinity,
            padding: const EdgeInsets.all(12),
            decoration: BoxDecoration(
              color: theme.colorScheme.surfaceContainerLow,
              borderRadius: BorderRadius.circular(14),
            ),
            child: Text(value?.isNotEmpty == true ? value! : '—'),
          ),
        ],
      ),
    );
  }
}
