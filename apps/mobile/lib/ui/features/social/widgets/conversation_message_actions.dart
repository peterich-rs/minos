import 'package:flutter/cupertino.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:minos/domain/social_message.dart';
import 'package:minos/ui/theme/theme.dart';

enum ConversationMessageAction { reply, copy, recall, retry }

/// Touch action sheet for a collaboration message (Desktop hover-bar analogue).
Future<ConversationMessageAction?> showConversationMessageActions(
  BuildContext context, {
  required SocialChatMessage message,
  required bool isMine,
}) {
  final canReply = message.canReply;
  final canRecall = isMine && message.canRecall;
  final canRetry =
      isMine && message.deliveryState == SocialMessageDeliveryState.failed;
  final canCopy = !message.isRecalled && message.text.trim().isNotEmpty;

  if (!canReply && !canRecall && !canRetry && !canCopy) {
    return Future<ConversationMessageAction?>.value(null);
  }

  return showModalBottomSheet<ConversationMessageAction>(
    context: context,
    useSafeArea: true,
    showDragHandle: true,
    builder: (sheetContext) {
      final colors = sheetContext.minosColors;
      return SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: <Widget>[
            if (canReply)
              ListTile(
                leading: Icon(CupertinoIcons.reply, color: colors.textPrimary),
                title: const Text('引用消息'),
                onTap: () => Navigator.of(
                  sheetContext,
                ).pop(ConversationMessageAction.reply),
              ),
            if (canCopy)
              ListTile(
                leading: Icon(
                  CupertinoIcons.doc_on_doc,
                  color: colors.textPrimary,
                ),
                title: const Text('复制'),
                onTap: () => Navigator.of(
                  sheetContext,
                ).pop(ConversationMessageAction.copy),
              ),
            if (canRetry)
              ListTile(
                leading: Icon(
                  CupertinoIcons.arrow_clockwise,
                  color: colors.danger,
                ),
                title: Text('重新发送', style: TextStyle(color: colors.danger)),
                onTap: () => Navigator.of(
                  sheetContext,
                ).pop(ConversationMessageAction.retry),
              ),
            if (canRecall)
              ListTile(
                leading: Icon(
                  CupertinoIcons.arrow_uturn_left,
                  color: colors.danger,
                ),
                title: Text('撤回消息', style: TextStyle(color: colors.danger)),
                onTap: () => Navigator.of(
                  sheetContext,
                ).pop(ConversationMessageAction.recall),
              ),
            const SizedBox(height: MinosSpacing.sm),
          ],
        ),
      );
    },
  );
}

Future<bool> confirmRecallMessage(BuildContext context) async {
  final colors = context.minosColors;
  final confirmed = await showCupertinoDialog<bool>(
    context: context,
    builder: (dialogContext) => CupertinoAlertDialog(
      title: const Text('撤回这条消息？'),
      content: const Text('撤回后，对话中会显示该消息已被撤回。'),
      actions: <Widget>[
        CupertinoDialogAction(
          onPressed: () => Navigator.of(dialogContext).pop(false),
          child: Text('取消', style: TextStyle(color: colors.textSecondary)),
        ),
        CupertinoDialogAction(
          isDefaultAction: true,
          onPressed: () => Navigator.of(dialogContext).pop(true),
          child: Text('撤回', style: TextStyle(color: colors.accent)),
        ),
      ],
    ),
  );
  return confirmed == true;
}

Future<void> copyMessageText(String text) async {
  await Clipboard.setData(ClipboardData(text: text));
}
