import 'package:flutter/material.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import 'package:minos/domain/active_session.dart';

/// Sticky bottom composer for the chat surface. Two visual states keyed
/// off the [ActiveSession]:
///
///   - Idle / AwaitingInput / Stopped → Send button (gated on
///     `_canSend`: text non-empty + ≤ [_maxChars]).
///   - Starting / Streaming → destructive Stop button.
///   - Error → Send retries; if the error has a thread id the parent resumes
///     that thread instead of starting a new agent.
///
/// The widget owns its own `TextEditingController`; the parent receives
/// the message via `onSend(text)` and is responsible for clearing /
/// resetting state by feeding back a new `session` value.
class InputBar extends StatefulWidget {
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
  State<InputBar> createState() => _InputBarState();
}

class _InputBarState extends State<InputBar> {
  static const int _maxChars = 8000;

  final TextEditingController _ctl = TextEditingController();
  final FocusNode _focus = FocusNode();

  @override
  void dispose() {
    _ctl.dispose();
    _focus.dispose();
    super.dispose();
  }

  bool get _isStreaming =>
      widget.session is SessionStreaming || widget.session is SessionStarting;

  bool get _canSend {
    final s = widget.session;
    final composable =
        s is SessionIdle ||
        s is SessionAwaitingInput ||
        s is SessionStopped ||
        s is SessionError;
    if (!composable) return false;
    final trimmed = _ctl.text.trim();
    if (trimmed.isEmpty) return false;
    if (_ctl.text.length > _maxChars) return false;
    return true;
  }

  void _submit() {
    if (!_canSend) return;
    final text = _ctl.text;
    widget.onSend(text);
    _ctl.clear();
    setState(() {});
  }

  @override
  Widget build(BuildContext context) {
    final shadTheme = ShadTheme.of(context);
    final materialTheme = Theme.of(context);
    final overLimit = _ctl.text.length > _maxChars;
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
            mainAxisSize: MainAxisSize.min,
            children: [
              ShadInput(
                controller: _ctl,
                focusNode: _focus,
                minLines: 1,
                maxLines: 6,
                enabled: !_isStreaming,
                textCapitalization: TextCapitalization.sentences,
                keyboardType: TextInputType.multiline,
                textInputAction: TextInputAction.newline,
                onChanged: (_) => setState(() {}),
                placeholder: const Text('继续追问，或让它帮你完成下一步...'),
                style: materialTheme.textTheme.bodyMedium?.copyWith(
                  height: 1.35,
                ),
                padding: const EdgeInsets.fromLTRB(12, 8, 8, 8),
                trailing: _ComposerActionButton(
                  icon: _isStreaming
                      ? LucideIcons.circleStop
                      : LucideIcons.sendHorizontal,
                  onTap: _isStreaming
                      ? widget.onStop
                      : (_canSend ? _submit : null),
                  destructive: _isStreaming,
                  enabled: _isStreaming || _canSend,
                ),
              ),
              const SizedBox(height: 7),
              Row(
                children: [
                  Expanded(
                    child: Text(
                      overLimit
                          ? '${_ctl.text.length} / $_maxChars 字符'
                          : helperText,
                      style: shadTheme.textTheme.muted.copyWith(
                        color: overLimit
                            ? shadTheme.colorScheme.destructive
                            : shadTheme.colorScheme.mutedForeground,
                      ),
                    ),
                  ),
                  if (!overLimit && !_isStreaming)
                    Icon(
                      LucideIcons.sparkles,
                      size: 14,
                      color: shadTheme.colorScheme.mutedForeground,
                    ),
                ],
              ),
            ],
          ),
        ),
      ),
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
