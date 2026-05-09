import 'dart:convert';
import 'dart:math';

import 'package:sqflite/sqflite.dart';

import 'package:minos/domain/social_message.dart';
import 'package:minos/infrastructure/app_paths.dart';
import 'package:minos/src/rust/api/minos.dart';

class SocialCacheStore {
  SocialCacheStore();

  static const _dbName = 'social_cache.db';
  static const _dbVersion = 2;

  Future<Database?>? _databaseFuture;

  Future<Database?> _database() {
    return _databaseFuture ??= _openDatabaseSafe();
  }

  Future<Database?> _openDatabaseSafe() async {
    try {
      final root = await minosAppDirectory();
      return openDatabase(
        '${root.path}/$_dbName',
        version: _dbVersion,
        onCreate: (db, version) async {
          await db.execute('''
            CREATE TABLE cached_social_conversations (
              conversation_id TEXT PRIMARY KEY,
              kind TEXT NOT NULL,
              title TEXT NOT NULL,
              counterpart_json TEXT,
              member_count INTEGER NOT NULL,
              last_message_preview TEXT,
              last_message_at_ms INTEGER NOT NULL,
              unread_count INTEGER NOT NULL,
              unread_mention_count INTEGER NOT NULL
            )
          ''');
          await db.execute('''
            CREATE TABLE cached_social_messages (
              local_id TEXT PRIMARY KEY,
              conversation_id TEXT NOT NULL,
              server_message_id TEXT UNIQUE,
              sender_json TEXT NOT NULL,
              text TEXT NOT NULL,
              created_at_ms INTEGER NOT NULL,
              client_seq INTEGER NOT NULL,
              server_order_key INTEGER,
              reply_to_message_id TEXT,
              reply_to_preview_json TEXT,
              recalled_at_ms INTEGER,
              mentioned_account_ids_json TEXT NOT NULL,
              delivery_state TEXT NOT NULL
            )
          ''');
          await db.execute(
            'CREATE INDEX idx_cached_social_messages_conversation ON cached_social_messages(conversation_id, client_seq)',
          );
          await db.execute('''
            CREATE TABLE cached_social_meta (
              key TEXT PRIMARY KEY,
              value TEXT NOT NULL
            )
          ''');
        },
        onUpgrade: (db, oldVersion, newVersion) async {
          if (oldVersion < 2) {
            await db.execute(
              'ALTER TABLE cached_social_messages ADD COLUMN reply_to_message_id TEXT',
            );
            await db.execute(
              'ALTER TABLE cached_social_messages ADD COLUMN reply_to_preview_json TEXT',
            );
            await db.execute(
              'ALTER TABLE cached_social_messages ADD COLUMN recalled_at_ms INTEGER',
            );
          }
        },
      );
    } catch (_) {
      return null;
    }
  }

  Future<void> saveCurrentAccountId(String accountId) async {
    final db = await _database();
    if (db == null) {
      return;
    }
    await db.insert('cached_social_meta', <String, Object?>{
      'key': 'current_account_id',
      'value': accountId,
    }, conflictAlgorithm: ConflictAlgorithm.replace);
  }

  Future<String?> loadCurrentAccountId() async {
    final db = await _database();
    if (db == null) {
      return null;
    }
    final rows = await db.query(
      'cached_social_meta',
      columns: const <String>['value'],
      where: 'key = ?',
      whereArgs: const <Object>['current_account_id'],
      limit: 1,
    );
    if (rows.isEmpty) {
      return null;
    }
    return rows.first['value'] as String?;
  }

  Future<ConversationsResponse?> loadConversations() async {
    final db = await _database();
    if (db == null) {
      return null;
    }
    final rows = await db.query(
      'cached_social_conversations',
      orderBy: 'last_message_at_ms DESC',
    );
    return ConversationsResponse(
      conversations: rows.map(_conversationFromRow).toList(growable: false),
    );
  }

  Future<void> saveConversations(
    List<ConversationSummary> conversations,
  ) async {
    final db = await _database();
    if (db == null) {
      return;
    }
    await db.transaction((txn) async {
      await txn.delete('cached_social_conversations');
      for (final conversation in conversations) {
        await txn.insert(
          'cached_social_conversations',
          _conversationToRow(conversation),
          conflictAlgorithm: ConflictAlgorithm.replace,
        );
      }
    });
  }

  Future<List<SocialChatMessage>> loadMessages(String conversationId) async {
    final db = await _database();
    if (db == null) {
      return const <SocialChatMessage>[];
    }
    final rows = await db.query(
      'cached_social_messages',
      where: 'conversation_id = ?',
      whereArgs: <Object>[conversationId],
      orderBy: 'COALESCE(server_order_key, created_at_ms) ASC, client_seq ASC',
    );
    return rows.map(_messageFromRow).toList(growable: false);
  }

  Future<void> upsertRemoteMessages({
    required String conversationId,
    required List<ChatMessageSummary> messages,
  }) async {
    final db = await _database();
    if (db == null) {
      return;
    }
    await db.transaction((txn) async {
      for (final message in messages) {
        await _upsertRemoteMessageTxn(txn, message);
      }
    });
  }

  Future<void> upsertRemoteMessage(ChatMessageSummary message) async {
    final db = await _database();
    if (db == null) {
      return;
    }
    await db.transaction((txn) async {
      await _upsertRemoteMessageTxn(txn, message);
    });
  }

  Future<SocialChatMessage> insertPendingMessage({
    required String conversationId,
    required UserSummary sender,
    required String text,
    ChatMessageReplySummary? replyTo,
  }) async {
    final localMessage = SocialChatMessage(
      localId: _newLocalId(),
      conversationId: conversationId,
      sender: sender,
      text: text,
      createdAtMs: DateTime.now().millisecondsSinceEpoch,
      clientSeq: DateTime.now().microsecondsSinceEpoch,
      deliveryState: SocialMessageDeliveryState.sending,
      replyTo: replyTo,
      mentionedAccountIds: const <String>[],
    );

    final db = await _database();
    if (db == null) {
      return localMessage;
    }

    final persisted = await db.transaction((txn) async {
      final nextClientSeq = await _nextClientSeq(txn, conversationId);
      final nextMessage = localMessage.copyWith(clientSeq: nextClientSeq);
      await txn.insert(
        'cached_social_messages',
        _messageToRow(nextMessage),
        conflictAlgorithm: ConflictAlgorithm.replace,
      );
      return nextMessage;
    });
    return persisted;
  }

  Future<SocialChatMessage?> markMessageSending(String localId) async {
    final db = await _database();
    if (db == null) {
      return null;
    }
    await db.update(
      'cached_social_messages',
      <String, Object?>{
        'delivery_state': SocialMessageDeliveryState.sending.name,
      },
      where: 'local_id = ?',
      whereArgs: <Object>[localId],
    );
    return _loadMessageByLocalId(db, localId);
  }

  Future<SocialChatMessage?> markMessageFailed(String localId) async {
    final db = await _database();
    if (db == null) {
      return null;
    }
    await db.update(
      'cached_social_messages',
      <String, Object?>{
        'delivery_state': SocialMessageDeliveryState.failed.name,
      },
      where: 'local_id = ?',
      whereArgs: <Object>[localId],
    );
    return _loadMessageByLocalId(db, localId);
  }

  Future<SocialChatMessage?> markMessageSent({
    required String localId,
    required ChatMessageSummary message,
  }) async {
    final db = await _database();
    if (db == null) {
      return null;
    }
    await db.transaction((txn) async {
      final existingRows = await txn.query(
        'cached_social_messages',
        where: 'local_id = ?',
        whereArgs: <Object>[localId],
        limit: 1,
      );
      if (existingRows.isEmpty) {
        await _upsertRemoteMessageTxn(txn, message);
        return;
      }
      final existing = _messageFromRow(existingRows.first);
      final duplicateRows = await txn.query(
        'cached_social_messages',
        columns: const <String>['local_id'],
        where: 'server_message_id = ? AND local_id != ?',
        whereArgs: <Object>[message.messageId, localId],
        limit: 1,
      );
      if (duplicateRows.isNotEmpty) {
        await txn.delete(
          'cached_social_messages',
          where: 'local_id = ?',
          whereArgs: <Object>[duplicateRows.first['local_id']! as String],
        );
      }
      await txn.update(
        'cached_social_messages',
        <String, Object?>{
          'conversation_id': message.conversationId,
          'server_message_id': message.messageId,
          'sender_json': jsonEncode(_userSummaryToMap(message.sender)),
          'text': message.text,
          'created_at_ms': message.createdAtMs,
          'server_order_key': message.createdAtMs,
          'reply_to_message_id': message.replyTo?.messageId,
          'reply_to_preview_json': message.replyTo == null
              ? null
              : jsonEncode(_replyPreviewToMap(message.replyTo!)),
          'recalled_at_ms': message.recalledAtMs,
          'mentioned_account_ids_json': jsonEncode(message.mentionedAccountIds),
          'delivery_state': SocialMessageDeliveryState.sent.name,
          'client_seq': existing.clientSeq,
        },
        where: 'local_id = ?',
        whereArgs: <Object>[localId],
      );
    });
    return _loadMessageByLocalId(db, localId);
  }

  Future<void> touchConversationPreview({
    required String conversationId,
    required String preview,
    required int createdAtMs,
  }) async {
    final db = await _database();
    if (db == null) {
      return;
    }
    await db.update(
      'cached_social_conversations',
      <String, Object?>{
        'last_message_preview': preview,
        'last_message_at_ms': createdAtMs,
      },
      where: 'conversation_id = ?',
      whereArgs: <Object>[conversationId],
    );
  }

  Future<int> _nextClientSeq(
    DatabaseExecutor executor,
    String conversationId,
  ) async {
    final rows = await executor.rawQuery(
      'SELECT COALESCE(MAX(client_seq), 0) AS max_seq FROM cached_social_messages WHERE conversation_id = ?',
      <Object>[conversationId],
    );
    final maxSeq = rows.first['max_seq'];
    return (maxSeq as int? ?? 0) + 1;
  }

  Future<void> _upsertRemoteMessageTxn(
    Transaction txn,
    ChatMessageSummary message,
  ) async {
    final existingRows = await txn.query(
      'cached_social_messages',
      where: 'server_message_id = ?',
      whereArgs: <Object>[message.messageId],
      limit: 1,
    );
    if (existingRows.isNotEmpty) {
      final existing = _messageFromRow(existingRows.first);
      await txn.update(
        'cached_social_messages',
        <String, Object?>{
          'conversation_id': message.conversationId,
          'sender_json': jsonEncode(_userSummaryToMap(message.sender)),
          'text': message.text,
          'created_at_ms': message.createdAtMs,
          'server_order_key': message.createdAtMs,
          'reply_to_message_id': message.replyTo?.messageId,
          'reply_to_preview_json': message.replyTo == null
              ? null
              : jsonEncode(_replyPreviewToMap(message.replyTo!)),
          'recalled_at_ms': message.recalledAtMs,
          'mentioned_account_ids_json': jsonEncode(message.mentionedAccountIds),
          'delivery_state': SocialMessageDeliveryState.sent.name,
          'client_seq': existing.clientSeq,
        },
        where: 'local_id = ?',
        whereArgs: <Object>[existing.localId],
      );
      return;
    }

    final nextClientSeq = await _nextClientSeq(txn, message.conversationId);
    await txn.insert('cached_social_messages', <String, Object?>{
      'local_id': 'srv:${message.messageId}',
      'conversation_id': message.conversationId,
      'server_message_id': message.messageId,
      'sender_json': jsonEncode(_userSummaryToMap(message.sender)),
      'text': message.text,
      'created_at_ms': message.createdAtMs,
      'client_seq': nextClientSeq,
      'server_order_key': message.createdAtMs,
      'reply_to_message_id': message.replyTo?.messageId,
      'reply_to_preview_json': message.replyTo == null
          ? null
          : jsonEncode(_replyPreviewToMap(message.replyTo!)),
      'recalled_at_ms': message.recalledAtMs,
      'mentioned_account_ids_json': jsonEncode(message.mentionedAccountIds),
      'delivery_state': SocialMessageDeliveryState.sent.name,
    }, conflictAlgorithm: ConflictAlgorithm.replace);
  }

  Future<SocialChatMessage?> _loadMessageByLocalId(
    Database db,
    String localId,
  ) async {
    final rows = await db.query(
      'cached_social_messages',
      where: 'local_id = ?',
      whereArgs: <Object>[localId],
      limit: 1,
    );
    if (rows.isEmpty) {
      return null;
    }
    return _messageFromRow(rows.first);
  }

  ConversationSummary _conversationFromRow(Map<String, Object?> row) {
    return ConversationSummary(
      conversationId: row['conversation_id']! as String,
      kind: ConversationKind.values.byName(row['kind']! as String),
      title: row['title']! as String,
      counterpart: _userSummaryFromJson(row['counterpart_json'] as String?),
      memberCount: row['member_count']! as int,
      lastMessagePreview: row['last_message_preview'] as String?,
      lastMessageAtMs: row['last_message_at_ms']! as int,
      unreadCount: row['unread_count']! as int,
      unreadMentionCount: row['unread_mention_count']! as int,
    );
  }

  Map<String, Object?> _conversationToRow(ConversationSummary conversation) {
    return <String, Object?>{
      'conversation_id': conversation.conversationId,
      'kind': conversation.kind.name,
      'title': conversation.title,
      'counterpart_json': conversation.counterpart == null
          ? null
          : jsonEncode(_userSummaryToMap(conversation.counterpart!)),
      'member_count': conversation.memberCount,
      'last_message_preview': conversation.lastMessagePreview,
      'last_message_at_ms': conversation.lastMessageAtMs,
      'unread_count': conversation.unreadCount,
      'unread_mention_count': conversation.unreadMentionCount,
    };
  }

  SocialChatMessage _messageFromRow(Map<String, Object?> row) {
    return SocialChatMessage(
      localId: row['local_id']! as String,
      conversationId: row['conversation_id']! as String,
      sender: _userSummaryFromMap(
        jsonDecode(row['sender_json']! as String) as Map<String, Object?>,
      ),
      text: row['text']! as String,
      createdAtMs: row['created_at_ms']! as int,
      clientSeq: row['client_seq']! as int,
      deliveryState: SocialMessageDeliveryState.values.byName(
        row['delivery_state']! as String,
      ),
      serverMessageId: row['server_message_id'] as String?,
      serverOrderKey: row['server_order_key'] as int?,
      replyTo: _replyPreviewFromJson(row['reply_to_preview_json'] as String?),
      recalledAtMs: row['recalled_at_ms'] as int?,
      mentionedAccountIds:
          (jsonDecode(row['mentioned_account_ids_json']! as String)
                  as List<dynamic>)
              .map((value) => value as String)
              .toList(growable: false),
    );
  }

  Map<String, Object?> _messageToRow(SocialChatMessage message) {
    return <String, Object?>{
      'local_id': message.localId,
      'conversation_id': message.conversationId,
      'server_message_id': message.serverMessageId,
      'sender_json': jsonEncode(_userSummaryToMap(message.sender)),
      'text': message.text,
      'created_at_ms': message.createdAtMs,
      'client_seq': message.clientSeq,
      'server_order_key': message.serverOrderKey,
      'reply_to_message_id': message.replyTo?.messageId,
      'reply_to_preview_json': message.replyTo == null
          ? null
          : jsonEncode(_replyPreviewToMap(message.replyTo!)),
      'recalled_at_ms': message.recalledAtMs,
      'mentioned_account_ids_json': jsonEncode(message.mentionedAccountIds),
      'delivery_state': message.deliveryState.name,
    };
  }

  Map<String, Object?> _replyPreviewToMap(ChatMessageReplySummary reply) {
    return <String, Object?>{
      'message_id': reply.messageId,
      'sender': _userSummaryToMap(reply.sender),
      'text': reply.text,
      'recalled_at_ms': reply.recalledAtMs,
    };
  }

  ChatMessageReplySummary? _replyPreviewFromJson(String? raw) {
    if (raw == null || raw.isEmpty) {
      return null;
    }
    final map = jsonDecode(raw) as Map<String, Object?>;
    return ChatMessageReplySummary(
      messageId: map['message_id']! as String,
      sender: _userSummaryFromMap(map['sender']! as Map<String, Object?>),
      text: map['text']! as String,
      recalledAtMs: map['recalled_at_ms'] as int?,
    );
  }

  Map<String, Object?> _userSummaryToMap(UserSummary user) {
    return <String, Object?>{
      'account_id': user.accountId,
      'minos_id': user.minosId,
      'display_name': user.displayName,
    };
  }

  UserSummary _userSummaryFromMap(Map<String, Object?> map) {
    return UserSummary(
      accountId: map['account_id']! as String,
      minosId: map['minos_id']! as String,
      displayName: map['display_name']! as String,
    );
  }

  UserSummary? _userSummaryFromJson(String? raw) {
    if (raw == null || raw.isEmpty) {
      return null;
    }
    return _userSummaryFromMap(jsonDecode(raw) as Map<String, Object?>);
  }

  String _newLocalId() {
    final random = Random.secure().nextInt(1 << 32);
    return 'local-${DateTime.now().microsecondsSinceEpoch}-$random';
  }
}
