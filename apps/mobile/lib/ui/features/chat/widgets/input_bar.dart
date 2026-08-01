import 'package:flutter/cupertino.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:minos/domain/active_session.dart';
import 'package:minos/ui/theme/theme.dart';

final _inputBarDraftProvider = NotifierProvider.autoDispose
    .family<_InputBarDraftController, String, String>(
      _InputBarDraftController.new,
    );

class _InputBarDraftController extends Notifier<String> {
  _InputBarDraftController(String _);

  @override
  String build() => '';

  void setText(String text) {
    if (state == text) return;
    state = text;
  }
}

/// Sticky bottom composer for the chat surface.
class InputBar extends ConsumerStatefulWidget {
  const InputBar({
    super.key,
    required this.session,
    required this.onSend,
    required this.onStop,
  });

  final ActiveSession session;
  final ValueChanged<String> onSend;
  final VoidCallback onStop;

  @override
  ConsumerState<InputBar> createState() => _InputBarState();
}

class _InputBarState extends ConsumerState<InputBar> {
  static const int _maxChars = 8000;

  final TextEditingController _ctl = TextEditingController();
  final FocusNode _focus = FocusNode();
  late final String _composerId = 'input-bar-${identityHashCode(this)}';

  @override
  void initState() {
    super.initState();
    _ctl.addListener(_syncDraftText);
  }

  @override
  void dispose() {
    _ctl.removeListener(_syncDraftText);
    _ctl.dispose();
    _focus.dispose();
    super.dispose();
  }

  void _syncDraftText() {
    ref.read(_inputBarDraftProvider(_composerId).notifier).setText(_ctl.text);
  }

  bool get _isStreaming =>
      widget.session is SessionStreaming || widget.session is SessionSending;

  bool _canSend(String draft) {
    final s = widget.session;
    final composable =
        s is SessionIdle ||
        s is SessionAwaitingInput ||
        s is SessionSuspended ||
        s is SessionError;
    if (!composable) return false;
    final trimmed = draft.trim();
    if (trimmed.isEmpty) return false;
    if (draft.length > _maxChars) return false;
    return true;
  }

  void _submit() {
    final text = _ctl.text;
    if (!_canSend(text)) return;
    widget.onSend(text);
    _ctl.clear();
  }

  @override
  Widget build(BuildContext context) {
    final colors = context.minosColors;
    final theme = Theme.of(context);
    final helperText = _isStreaming ? 'Agent 正在回复，可随时停止。' : '准备好后发送，可连续追问。';

    return SafeArea(
      top: false,
      child: DecoratedBox(
        decoration: BoxDecoration(
          color: colors.surface,
          border: Border(top: BorderSide(color: colors.borderSubtle)),
        ),
        child: Padding(
          padding: const EdgeInsets.fromLTRB(
            MinosSpacing.md,
            MinosSpacing.md,
            MinosSpacing.md,
            MinosSpacing.md,
          ),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: <Widget>[
              Container(
                decoration: BoxDecoration(
                  color: colors.surfaceMuted,
                  borderRadius: MinosRadii.mdAll,
                  border: Border.all(color: colors.borderSubtle),
                ),
                padding: const EdgeInsets.fromLTRB(12, 4, 4, 4),
                child: Row(
                  crossAxisAlignment: CrossAxisAlignment.end,
                  children: <Widget>[
                    Expanded(
                      child: TextField(
                        controller: _ctl,
                        focusNode: _focus,
                        minLines: 1,
                        maxLines: 6,
                        enabled: !_isStreaming,
                        textCapitalization: TextCapitalization.sentences,
                        keyboardType: TextInputType.multiline,
                        textInputAction: TextInputAction.newline,
                        style: theme.textTheme.bodyMedium?.copyWith(
                          height: 1.35,
                        ),
                        decoration: InputDecoration(
                          hintText: '继续追问，或让它帮你完成下一步…',
                          hintStyle: theme.textTheme.bodyMedium?.copyWith(
                            color: colors.textTertiary,
                          ),
                          border: InputBorder.none,
                          enabledBorder: InputBorder.none,
                          focusedBorder: InputBorder.none,
                          filled: false,
                          contentPadding: const EdgeInsets.symmetric(
                            vertical: 10,
                          ),
                        ),
                      ),
                    ),
                    Consumer(
                      builder: (context, ref, _) {
                        final draft = ref.watch(
                          _inputBarDraftProvider(_composerId),
                        );
                        final canSend = _canSend(draft);
                        return _ComposerActionButton(
                          icon: _isStreaming
                              ? CupertinoIcons.stop_circle_fill
                              : CupertinoIcons.arrow_up_circle_fill,
                          onTap: _isStreaming
                              ? widget.onStop
                              : (canSend ? _submit : null),
                          destructive: _isStreaming,
                          enabled: _isStreaming || canSend,
                        );
                      },
                    ),
                  ],
                ),
              ),
              const SizedBox(height: MinosSpacing.sm),
              _ComposerStatusRow(
                composerId: _composerId,
                isStreaming: _isStreaming,
                helperText: helperText,
                maxChars: _maxChars,
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _ComposerStatusRow extends ConsumerWidget {
  const _ComposerStatusRow({
    required this.composerId,
    required this.isStreaming,
    required this.helperText,
    required this.maxChars,
  });

  final String composerId;
  final bool isStreaming;
  final String helperText;
  final int maxChars;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final colors = context.minosColors;
    final draftLength = ref.watch(
      _inputBarDraftProvider(composerId).select((draft) => draft.length),
    );
    final overLimit = draftLength > maxChars;

    return Row(
      children: <Widget>[
        Expanded(
          child: Text(
            overLimit ? '$draftLength / $maxChars 字符' : helperText,
            style: Theme.of(context).textTheme.bodySmall?.copyWith(
              color: overLimit ? colors.danger : colors.textTertiary,
            ),
          ),
        ),
        if (!overLimit && !isStreaming)
          Icon(CupertinoIcons.sparkles, size: 14, color: colors.textTertiary),
      ],
    );
  }
}

class _ComposerActionButton extends StatelessWidget {
  const _ComposerActionButton({
    required this.icon,
    required this.onTap,
    required this.destructive,
    required this.enabled,
  });

  final IconData icon;
  final VoidCallback? onTap;
  final bool destructive;
  final bool enabled;

  @override
  Widget build(BuildContext context) {
    final colors = context.minosColors;
    final color = !enabled
        ? colors.textTertiary.withValues(alpha: 0.45)
        : (destructive ? colors.danger : colors.accent);

    return IconButton(
      onPressed: enabled ? onTap : null,
      icon: Icon(icon, color: color, size: 30),
      visualDensity: VisualDensity.compact,
      padding: EdgeInsets.zero,
      constraints: const BoxConstraints(minWidth: 40, minHeight: 40),
    );
  }
}
