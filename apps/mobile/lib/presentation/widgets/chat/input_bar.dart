import 'package:flutter/material.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import 'package:minos/domain/active_session.dart';

/// Sticky bottom composer for the chat surface. Two visual states keyed
/// off the [ActiveSession]:
///
///   - Idle / AwaitingInput / Stopped → Send button (gated on
///     `_canSend`: text non-empty + ≤ [_maxChars]).
///   - Starting / Streaming → destructive Stop button.
///   - Error → both inputs disabled (caller can offer a retry above).
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
        (s is SessionError && s.threadId == null);
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
    final theme = Theme.of(context);
    final overLimit = _ctl.text.length > _maxChars;
    return SafeArea(
      top: false,
      child: Container(
        padding: const EdgeInsets.fromLTRB(8, 6, 8, 8),
        decoration: BoxDecoration(
          color: theme.colorScheme.surface,
          border: Border(
            top: BorderSide(color: theme.colorScheme.outlineVariant),
          ),
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Row(
              crossAxisAlignment: CrossAxisAlignment.end,
              children: [
                Expanded(
                  child: ShadInput(
                    controller: _ctl,
                    focusNode: _focus,
                    placeholder: const Text('输入消息…'),
                    minLines: 1,
                    maxLines: 4,
                    enabled: !_isStreaming,
                    onChanged: (_) => setState(() {}),
                  ),
                ),
                const SizedBox(width: 8),
                if (_isStreaming)
                  ShadButton.destructive(
                    onPressed: widget.onStop,
                    child: const Text('停止'),
                  )
                else
                  ShadButton(
                    enabled: _canSend,
                    onPressed: _canSend ? _submit : null,
                    child: const Text('发送'),
                  ),
              ],
            ),
            if (overLimit)
              Padding(
                padding: const EdgeInsets.only(top: 4),
                child: Align(
                  alignment: Alignment.centerLeft,
                  child: Text(
                    '${_ctl.text.length} / $_maxChars 字符',
                    style: theme.textTheme.labelSmall?.copyWith(
                      color: theme.colorScheme.error,
                    ),
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }
}
