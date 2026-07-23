import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:minos/domain/active_session.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

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

/// Sticky bottom composer for the chat surface. Two visual states keyed
/// off the [ActiveSession]:
///
///   - Idle / AwaitingInput / Stopped → Send button (gated on
///     `_canSend`: text non-empty + ≤ [_maxChars]).
///   - Starting / Streaming → destructive Stop button.
///   - Error → Send retries; if the error has a session id the parent resumes
///     that session instead of starting a new agent.
///
/// The widget owns its own `TextEditingController`; the parent receives
/// the message via `onSend(text)` and is responsible for clearing /
/// resetting state by feeding back a new `session` value.
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
    final shadTheme = ShadTheme.of(context);
    final materialTheme = Theme.of(context);
    final helperText = _isStreaming ? 'Agent 正在回复，可随时停止。' : '准备好后发送，可连续追问。';
    return SafeArea(
      top: false,
      child: DecoratedBox(
        decoration: BoxDecoration(
          color: shadTheme.colorScheme.background,
          border: Border(top: BorderSide(color: shadTheme.colorScheme.border)),
        ),
        child: Padding(
          padding: const EdgeInsets.fromLTRB(12, 10, 12, 10),
          child: Column(
            mainAxisSize: .min,
            children: [
              ShadInput(
                controller: _ctl,
                focusNode: _focus,
                minLines: 1,
                maxLines: 6,
                enabled: !_isStreaming,
                textCapitalization: .sentences,
                keyboardType: .multiline,
                textInputAction: .newline,
                placeholder: const Text('继续追问，或让它帮你完成下一步...'),
                style: materialTheme.textTheme.bodyMedium?.copyWith(
                  height: 1.35,
                ),
                padding: const EdgeInsets.fromLTRB(12, 8, 8, 8),
                trailing: Consumer(
                  builder: (context, ref, _) {
                    final draft = ref.watch(
                      _inputBarDraftProvider(_composerId),
                    );
                    final canSend = _canSend(draft);
                    return _ComposerActionButton(
                      icon: _isStreaming
                          ? LucideIcons.circleStop
                          : LucideIcons.sendHorizontal,
                      onTap: _isStreaming
                          ? widget.onStop
                          : (canSend ? _submit : null),
                      destructive: _isStreaming,
                      enabled: _isStreaming || canSend,
                    );
                  },
                ),
              ),
              const SizedBox(height: 7),
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
    final shadTheme = ShadTheme.of(context);
    final draftLength = ref.watch(
      _inputBarDraftProvider(composerId).select((draft) => draft.length),
    );
    final overLimit = draftLength > maxChars;

    return Row(
      children: [
        Expanded(
          child: Text(
            overLimit ? '$draftLength / $maxChars 字符' : helperText,
            style: shadTheme.textTheme.muted.copyWith(
              color: overLimit
                  ? shadTheme.colorScheme.destructive
                  : shadTheme.colorScheme.mutedForeground,
            ),
          ),
        ),
        if (!overLimit && !isStreaming)
          Icon(
            LucideIcons.sparkles,
            size: 14,
            color: shadTheme.colorScheme.mutedForeground,
          ),
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
    final button = destructive
        ? ShadIconButton.destructive(
            icon: Icon(icon),
            iconSize: 18,
            width: 36,
            height: 36,
            enabled: enabled,
            onPressed: onTap,
          )
        : ShadIconButton(
            icon: Icon(icon),
            iconSize: 18,
            width: 36,
            height: 36,
            enabled: enabled,
            onPressed: onTap,
          );
    return AnimatedOpacity(
      duration: const Duration(milliseconds: 160),
      opacity: enabled ? 1 : 0.6,
      child: button,
    );
  }
}
