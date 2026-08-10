import 'package:minos/src/rust/api/minos.dart';

/// Helpers for wire [MessageSender] (Account | Bot).
extension MessageSenderExt on MessageSender {
  /// Primary identity id: account_id or bot_id.
  String get identityId => switch (this) {
        MessageSender_Account(:final accountId) => accountId,
        MessageSender_Bot(:final botId) => botId,
      };

  /// Account principal when human; null for bots.
  String? get accountIdOrNull => switch (this) {
        MessageSender_Account(:final accountId) => accountId,
        MessageSender_Bot() => null,
      };

  /// Bot id when bot; null for humans.
  String? get botIdOrNull => switch (this) {
        MessageSender_Bot(:final botId) => botId,
        MessageSender_Account() => null,
      };

  String get minosIdOrEmpty => switch (this) {
        MessageSender_Account(:final minosId) => minosId,
        MessageSender_Bot(:final name, :final botId) =>
          (name != null && name.trim().isNotEmpty) ? name.trim() : botId,
      };

  bool get isBot => this is MessageSender_Bot;
  bool get isAccount => this is MessageSender_Account;

  /// Grouping / ownership key used by timeline UI.
  String get groupingKey => switch (this) {
        MessageSender_Account(:final accountId) => 'user:$accountId',
        MessageSender_Bot(:final botId) => 'bot:$botId',
      };
}

/// Serialize [MessageSender] for local SQLite cache (latest-only shape).
Map<String, Object?> messageSenderToMap(MessageSender sender) {
  return switch (sender) {
    MessageSender_Account(
      :final accountId,
      :final minosId,
      :final displayName,
    ) =>
      <String, Object?>{
        'kind': 'account',
        'account_id': accountId,
        'minos_id': minosId,
        'display_name': displayName,
      },
    MessageSender_Bot(
      :final botId,
      :final displayName,
      :final runtimeAgent,
      :final name,
      :final avatarUrl,
    ) =>
      <String, Object?>{
        'kind': 'bot',
        'bot_id': botId,
        'display_name': displayName,
        'runtime_agent': runtimeAgent,
        'name': ?name,
        'avatar_url': ?avatarUrl,
      },
  };
}

/// Deserialize cache / wire map into [MessageSender].
///
/// Accepts the new tagged shape and the legacy flat UserSummary map.
MessageSender messageSenderFromMap(Map<String, Object?> map) {
  final kind = (map['kind'] as String?)?.trim();
  if (kind == 'bot' || map.containsKey('bot_id')) {
    return MessageSender.bot(
      botId: (map['bot_id'] as String?)?.trim().isNotEmpty == true
          ? map['bot_id']! as String
          : (map['account_id'] as String? ?? ''),
      displayName: map['display_name'] as String? ?? '',
      runtimeAgent: map['runtime_agent'] as String? ?? '',
      name: map['name'] as String?,
      avatarUrl: map['avatar_url'] as String?,
    );
  }
  // kind == account | legacy UserSummary
  return MessageSender.account(
    accountId: map['account_id'] as String? ?? '',
    minosId: map['minos_id'] as String? ?? '',
    displayName: map['display_name'] as String? ?? '',
  );
}
