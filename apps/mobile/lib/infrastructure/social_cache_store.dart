import 'dart:convert';
import 'dart:math';

import 'package:minos/domain/message_sender_ext.dart';
import 'package:minos/domain/social_message.dart';
import 'package:minos/domain/social_message_order.dart';
import 'package:minos/infrastructure/app_paths.dart';
import 'package:minos/infrastructure/im_outbox_store.dart';
import 'package:minos/infrastructure/platform_int64.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:sqflite/sqflite.dart';

class SocialCacheStore {
  SocialCacheStore();

  static const _dbName = 'social_cache.db';
  static const _dbVersion = 7;

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
              client_message_id TEXT,
              sender_json TEXT NOT NULL,
              text TEXT NOT NULL,
              created_at_ms INTEGER NOT NULL,
              client_seq INTEGER NOT NULL,
              server_order_key INTEGER,
              sender_type TEXT NOT NULL DEFAULT 'user',
              reply_to_message_id TEXT,
              reply_to_preview_json TEXT,
              recalled_at_ms INTEGER,
              mentioned_account_ids_json TEXT NOT NULL,
              mentioned_agent_ids_json TEXT NOT NULL DEFAULT '[]',
              delivery_state TEXT NOT NULL,
              reactions_json TEXT NOT NULL DEFAULT '[]'
            )
          ''');
          await db.execute(
            'CREATE INDEX idx_cached_social_messages_conversation ON cached_social_messages(conversation_id, client_seq)',
          );
          // Durable timeline index: seq primary, then client_seq (no COALESCE with ms).
          await db.execute(
            'CREATE INDEX idx_cached_social_messages_conv_seq ON cached_social_messages(conversation_id, server_order_key, client_seq)',
          );
          await db.execute(
            'CREATE INDEX idx_cached_social_messages_client_message_id ON cached_social_messages(client_message_id)',
          );
          await db.execute('''
            CREATE TABLE cached_social_meta (
              key TEXT PRIMARY KEY,
              value TEXT NOT NULL
            )
          ''');
          await _createImOutboxTable(db);
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
          if (oldVersion < 3) {
            await db.execute(
              "ALTER TABLE cached_social_messages ADD COLUMN sender_type TEXT NOT NULL DEFAULT 'user'",
            );
          }
          if (oldVersion < 4) {
            await db.execute(
              'ALTER TABLE cached_social_messages ADD COLUMN client_message_id TEXT',
            );
            await db.execute(
              'CREATE INDEX IF NOT EXISTS idx_cached_social_messages_client_message_id ON cached_social_messages(client_message_id)',
            );
            await _createImOutboxTable(db);
            // Backfill: pending/sending local ids become client_message_id.
            await db.execute(
              'UPDATE cached_social_messages SET client_message_id = local_id WHERE client_message_id IS NULL',
            );
          }
          if (oldVersion < 5) {
            await db.execute(
              'CREATE INDEX IF NOT EXISTS idx_cached_social_messages_conv_seq ON cached_social_messages(conversation_id, server_order_key, client_seq)',
            );
          }
          if (oldVersion < 6) {
            await db.execute(
              "ALTER TABLE cached_social_messages ADD COLUMN reactions_json TEXT NOT NULL DEFAULT '[]'",
            );
          }
          if (oldVersion < 7) {
            await db.execute(
              "ALTER TABLE cached_social_messages ADD COLUMN mentioned_agent_ids_json TEXT NOT NULL DEFAULT '[]'",
            );
          }
        },
      );
    } catch (_) {
      return null;
    }
  }

  Future<void> _createImOutboxTable(DatabaseExecutor db) async {
    await db.execute('''
      CREATE TABLE IF NOT EXISTS im_outbox (
        client_op_id TEXT PRIMARY KEY,
        kind TEXT NOT NULL,
        conversation_id TEXT NOT NULL,
        payload_json TEXT NOT NULL,
        status TEXT NOT NULL,
        attempts INTEGER NOT NULL DEFAULT 0,
        next_attempt_at_ms INTEGER NOT NULL,
        last_error TEXT,
        created_at_ms INTEGER NOT NULL,
        updated_at_ms INTEGER NOT NULL
      )
    ''');
    await db.execute(
      'CREATE INDEX IF NOT EXISTS idx_im_outbox_due ON im_outbox(status, next_attempt_at_ms)',
    );
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

  /// Full hydrate / refresh only — **not** the inbound hot path.
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

  /// Single-row inbox upsert (InboxSync hot path).
  Future<void> upsertConversation(ConversationSummary conversation) async {
    final db = await _database();
    if (db == null) {
      return;
    }
    await db.insert(
      'cached_social_conversations',
      _conversationToRow(conversation),
      conflictAlgorithm: ConflictAlgorithm.replace,
    );
  }

  /// Bump unread counters for a background conversation (non-focused inbound).
  Future<ConversationSummary?> bumpUnread(
    String conversationId, {
    int unreadDelta = 1,
    bool mention = false,
  }) async {
    final db = await _database();
    if (db == null) {
      return null;
    }
    final rows = await db.query(
      'cached_social_conversations',
      where: 'conversation_id = ?',
      whereArgs: <Object>[conversationId],
      limit: 1,
    );
    if (rows.isEmpty) {
      return null;
    }
    final prev = _conversationFromRow(rows.first);
    final next = ConversationSummary(
      conversationId: prev.conversationId,
      kind: prev.kind,
      title: prev.title,
      counterpart: prev.counterpart,
      memberCount: prev.memberCount,
      lastMessagePreview: prev.lastMessagePreview,
      lastMessageAtMs: prev.lastMessageAtMs,
      unreadCount: (prev.unreadCount + unreadDelta).clamp(0, 1 << 30),
      unreadMentionCount: mention
          ? (prev.unreadMentionCount + 1).clamp(0, 1 << 30)
          : prev.unreadMentionCount,
    );
    await db.insert(
      'cached_social_conversations',
      _conversationToRow(next),
      conflictAlgorithm: ConflictAlgorithm.replace,
    );
    return next;
  }

  Future<void> clearUnread(String conversationId) async {
    final db = await _database();
    if (db == null) {
      return;
    }
    await db.update(
      'cached_social_conversations',
      <String, Object?>{'unread_count': 0, 'unread_mention_count': 0},
      where: 'conversation_id = ?',
      whereArgs: <Object>[conversationId],
    );
  }

  Future<ConversationSummary?> loadConversation(String conversationId) async {
    final db = await _database();
    if (db == null) {
      return null;
    }
    final rows = await db.query(
      'cached_social_conversations',
      where: 'conversation_id = ?',
      whereArgs: <Object>[conversationId],
      limit: 1,
    );
    if (rows.isEmpty) {
      return null;
    }
    return _conversationFromRow(rows.first);
  }

  Future<void> deleteConversation(String conversationId) async {
    final db = await _database();
    if (db == null) {
      return;
    }
    await db.transaction((txn) async {
      await txn.delete(
        'cached_social_messages',
        where: 'conversation_id = ?',
        whereArgs: <Object>[conversationId],
      );
      await txn.delete(
        'cached_social_conversations',
        where: 'conversation_id = ?',
        whereArgs: <Object>[conversationId],
      );
    });
  }

  Future<List<SocialChatMessage>> loadMessages(String conversationId) async {
    final db = await _database();
    if (db == null) {
      return const <SocialChatMessage>[];
    }
    // SQL order is a best-effort index path; canonical order is seq primary
    // for durable rows, optimistic-without-seq at tail. Never
    // COALESCE(server_order_key, created_at_ms) — different dimensions.
    final rows = await db.query(
      'cached_social_messages',
      where: 'conversation_id = ?',
      whereArgs: <Object>[conversationId],
      orderBy:
          'CASE WHEN server_order_key IS NULL THEN 1 ELSE 0 END ASC, '
          'server_order_key ASC, client_seq ASC',
    );
    // Dedupe by local_id so list keys stay unique.
    final byLocalId = <String, SocialChatMessage>{};
    for (final row in rows) {
      final message = _messageFromRow(row);
      byLocalId[message.localId] = message;
    }
    return sortSocialChatMessages(byLocalId.values);
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

  /// Atomically insert optimistic pending message + outbox row.
  ///
  /// One SQLite TX so a crash cannot leave a sending message without an outbox
  /// cover (or an outbox without a local row). [reconcileSendingMessagesOnStartup]
  /// remains the safety net for stranded sending rows.
  Future<SocialChatMessage> insertPendingMessageWithOutbox({
    required String conversationId,
    required MessageSender sender,
    required String text,
    ChatMessageReplySummary? replyTo,
    List<String> mentionedAccountIds = const <String>[],
    List<String> mentionedAgentIds = const <String>[],
    List<Map<String, Object?>> structuredMentions = const <Map<String, Object?>>[],
  }) async {
    // client_message_id == localId: stable wire id for Hub idempotent insert.
    final clientMessageId = _newClientMessageId();
    final localMessage = SocialChatMessage(
      localId: clientMessageId,
      conversationId: conversationId,
      sender: sender,
      text: text,
      createdAtMs: DateTime.now().millisecondsSinceEpoch,
      clientSeq: DateTime.now().microsecondsSinceEpoch,
      deliveryState: SocialMessageDeliveryState.sending,
      clientMessageId: clientMessageId,
      replyTo: replyTo,
      // Optimistic seed: callers pass best-effort parse so reload before ack
      // still shows structured bot mentions (Hub upsert remains SSOT).
      mentionedAccountIds: List<String>.unmodifiable(mentionedAccountIds),
      mentionedAgentIds: List<String>.unmodifiable(mentionedAgentIds),
    );
    final payloadJson = _userMessageOutboxPayload(
      text: text,
      replyToMessageId: replyTo?.messageId,
      structuredMentions: structuredMentions,
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
      await _enqueueOutboxTxn(
        txn,
        clientOpId: clientMessageId,
        kind: ImOutboxKind.userMessage,
        conversationId: conversationId,
        payloadJson: payloadJson,
      );
      return nextMessage;
    });
    return persisted;
  }

  // ── IM Outbox ────────────────────────────────────────────────────────

  /// Re-queue a failed user message (same client_message_id). Prefer
  /// [insertPendingMessageWithOutbox] for first send.
  Future<void> enqueueUserMessageOutbox({
    required String clientMessageId,
    required String conversationId,
    required String text,
    String? replyToMessageId,
    List<Map<String, Object?>> structuredMentions = const <Map<String, Object?>>[],
  }) async {
    final payload = _userMessageOutboxPayload(
      text: text,
      replyToMessageId: replyToMessageId,
      structuredMentions: structuredMentions,
    );
    await _enqueueOutbox(
      clientOpId: clientMessageId,
      kind: ImOutboxKind.userMessage,
      conversationId: conversationId,
      payloadJson: payload,
    );
  }

  String _userMessageOutboxPayload({
    required String text,
    String? replyToMessageId,
    List<Map<String, Object?>> structuredMentions = const <Map<String, Object?>>[],
  }) {
    return jsonEncode(<String, Object?>{
      'text': text,
      'reply_to_message_id': replyToMessageId,
      if (structuredMentions.isNotEmpty) 'mentions': structuredMentions,
    });
  }

  /// Enqueue reaction toggle; `clientOpId` is the wire client_op_id.
  Future<void> enqueueReactionToggleOutbox({
    required String clientOpId,
    required String conversationId,
    required String messageId,
    required String emoji,
  }) async {
    final payload = jsonEncode(<String, Object?>{
      'message_id': messageId,
      'emoji': emoji,
    });
    await _enqueueOutbox(
      clientOpId: clientOpId,
      kind: ImOutboxKind.reactionToggle,
      conversationId: conversationId,
      payloadJson: payload,
    );
  }

  Future<void> _enqueueOutbox({
    required String clientOpId,
    required ImOutboxKind kind,
    required String conversationId,
    required String payloadJson,
  }) async {
    final db = await _database();
    if (db == null) {
      return;
    }
    await db.transaction((txn) async {
      await _enqueueOutboxTxn(
        txn,
        clientOpId: clientOpId,
        kind: kind,
        conversationId: conversationId,
        payloadJson: payloadJson,
      );
    });
  }

  Future<void> _enqueueOutboxTxn(
    DatabaseExecutor txn, {
    required String clientOpId,
    required ImOutboxKind kind,
    required String conversationId,
    required String payloadJson,
  }) async {
    final now = DateTime.now().millisecondsSinceEpoch;
    final existing = await txn.query(
      'im_outbox',
      where: 'client_op_id = ?',
      whereArgs: <Object>[clientOpId],
      limit: 1,
    );
    if (existing.isNotEmpty) {
      final status = imOutboxStatusFromWire(
        existing.first['status']! as String,
      );
      if (status == ImOutboxStatus.acked) {
        return;
      }
      await txn.update(
        'im_outbox',
        <String, Object?>{
          'payload_json': payloadJson,
          'kind': imOutboxKindWire(kind),
          'status': imOutboxStatusWire(ImOutboxStatus.pending),
          'next_attempt_at_ms': now,
          'updated_at_ms': now,
          'last_error': null,
        },
        where: 'client_op_id = ?',
        whereArgs: <Object>[clientOpId],
      );
      return;
    }
    await txn.insert('im_outbox', <String, Object?>{
      'client_op_id': clientOpId,
      'kind': imOutboxKindWire(kind),
      'conversation_id': conversationId,
      'payload_json': payloadJson,
      'status': imOutboxStatusWire(ImOutboxStatus.pending),
      'attempts': 0,
      'next_attempt_at_ms': now,
      'last_error': null,
      'created_at_ms': now,
      'updated_at_ms': now,
    });
  }

  Future<int> reclaimStaleOutbox({int? nowMs}) async {
    final db = await _database();
    if (db == null) {
      return 0;
    }
    final now = nowMs ?? DateTime.now().millisecondsSinceEpoch;
    final cutoff = now - kImOutboxStaleInflightMs;
    return db.update(
      'im_outbox',
      <String, Object?>{
        'status': imOutboxStatusWire(ImOutboxStatus.pending),
        'next_attempt_at_ms': now,
        'updated_at_ms': now,
        'last_error': 'stale_inflight_reclaimed',
      },
      where: "status = ? AND updated_at_ms < ?",
      whereArgs: <Object>[imOutboxStatusWire(ImOutboxStatus.inflight), cutoff],
    );
  }

  Future<List<ImOutboxEntry>> listDueOutbox({int? nowMs}) async {
    final lanes = await listDueOutboxLanes(nowMs: nowMs);
    return lanes.expand((lane) => lane).toList(growable: false);
  }

  /// Per-conversation FIFO send lanes whose head is currently due.
  ///
  /// Head blocked (backoff / inflight) → whole lane omitted so tails cannot
  /// overtake. Contiguous due-pending prefix is returned per conversation.
  Future<List<List<ImOutboxEntry>>> listDueOutboxLanes({int? nowMs}) async {
    final db = await _database();
    if (db == null) {
      return const <List<ImOutboxEntry>>[];
    }
    final now = nowMs ?? DateTime.now().millisecondsSinceEpoch;
    await reclaimStaleOutbox(nowMs: now);
    // Active rows only (pending + inflight) so a backoff head blocks the lane.
    final rows = await db.query(
      'im_outbox',
      where: 'status = ? OR status = ?',
      whereArgs: <Object>[
        imOutboxStatusWire(ImOutboxStatus.pending),
        imOutboxStatusWire(ImOutboxStatus.inflight),
      ],
      orderBy: 'created_at_ms ASC, client_op_id ASC',
    );
    final active = rows.map(_outboxFromRow).toList(growable: false);
    return buildDueOutboxLanes(activeEntries: active, nowMs: now);
  }

  Future<void> markOutboxInflight(String clientOpId) async {
    final db = await _database();
    if (db == null) {
      return;
    }
    final now = DateTime.now().millisecondsSinceEpoch;
    await db.rawUpdate(
      '''
      UPDATE im_outbox
         SET status = ?,
             attempts = attempts + 1,
             updated_at_ms = ?
       WHERE client_op_id = ? AND status != ?
      ''',
      <Object>[
        imOutboxStatusWire(ImOutboxStatus.inflight),
        now,
        clientOpId,
        imOutboxStatusWire(ImOutboxStatus.acked),
      ],
    );
  }

  Future<void> markOutboxAcked(String clientOpId) async {
    final db = await _database();
    if (db == null) {
      return;
    }
    final now = DateTime.now().millisecondsSinceEpoch;
    await db.update(
      'im_outbox',
      <String, Object?>{
        'status': imOutboxStatusWire(ImOutboxStatus.acked),
        'updated_at_ms': now,
        'last_error': null,
      },
      where: 'client_op_id = ?',
      whereArgs: <Object>[clientOpId],
    );
  }

  Future<void> markOutboxFailed({
    required String clientOpId,
    required String error,
  }) async {
    final db = await _database();
    if (db == null) {
      return;
    }
    final rows = await db.query(
      'im_outbox',
      where: 'client_op_id = ?',
      whereArgs: <Object>[clientOpId],
      limit: 1,
    );
    if (rows.isEmpty) {
      return;
    }
    final attempts = rows.first['attempts']! as int;
    final now = DateTime.now().millisecondsSinceEpoch;
    final outcome = resolveOutboxFailure(
      attempts: attempts,
      error: error,
      nowMs: now,
    );
    await db.update(
      'im_outbox',
      <String, Object?>{
        'status': imOutboxStatusWire(outcome.status),
        'next_attempt_at_ms': outcome.nextAttemptAtMs,
        'updated_at_ms': now,
        'last_error': error,
      },
      where: 'client_op_id = ?',
      whereArgs: <Object>[clientOpId],
    );
    // Only surface UI "failed" when permanently terminal (not long offline).
    if (outcome.status == ImOutboxStatus.failedTerminal) {
      await markMessageFailed(clientOpId);
    }
  }

  /// Startup: reclaim inflight outbox + flip stranded sending rows to failed
  /// only when no pending/inflight outbox covers them.
  Future<void> reconcileSendingMessagesOnStartup() async {
    final db = await _database();
    if (db == null) {
      return;
    }
    await reclaimStaleOutbox();
    // Any remaining inflight (non-stale) from prior process → pending.
    final now = DateTime.now().millisecondsSinceEpoch;
    await db.update(
      'im_outbox',
      <String, Object?>{
        'status': imOutboxStatusWire(ImOutboxStatus.pending),
        'next_attempt_at_ms': now,
        'updated_at_ms': now,
      },
      where: 'status = ?',
      whereArgs: <Object>[imOutboxStatusWire(ImOutboxStatus.inflight)],
    );
    // Messages stuck in sending without outbox row → failed (manual retry).
    await db.rawUpdate(
      '''
      UPDATE cached_social_messages
         SET delivery_state = ?
       WHERE delivery_state = ?
         AND local_id NOT IN (
           SELECT client_op_id FROM im_outbox
            WHERE status IN (?, ?)
         )
      ''',
      <Object>[
        SocialMessageDeliveryState.failed.name,
        SocialMessageDeliveryState.sending.name,
        imOutboxStatusWire(ImOutboxStatus.pending),
        imOutboxStatusWire(ImOutboxStatus.inflight),
      ],
    );
  }

  Future<SocialChatMessage?> loadMessageByClientMessageId(
    String clientMessageId,
  ) async {
    final db = await _database();
    if (db == null) {
      return null;
    }
    final rows = await db.query(
      'cached_social_messages',
      where: 'client_message_id = ? OR local_id = ?',
      whereArgs: <Object>[clientMessageId, clientMessageId],
      limit: 1,
    );
    if (rows.isEmpty) {
      return null;
    }
    return _messageFromRow(rows.first);
  }

  ImOutboxEntry _outboxFromRow(Map<String, Object?> row) {
    return ImOutboxEntry(
      clientOpId: row['client_op_id']! as String,
      kind: imOutboxKindFromWire(row['kind']! as String),
      conversationId: row['conversation_id']! as String,
      payloadJson: row['payload_json']! as String,
      status: imOutboxStatusFromWire(row['status']! as String),
      attempts: row['attempts']! as int,
      nextAttemptAtMs: row['next_attempt_at_ms']! as int,
      lastError: row['last_error'] as String?,
      createdAtMs: row['created_at_ms']! as int,
      updatedAtMs: row['updated_at_ms']! as int,
    );
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
          'client_message_id': existing.clientMessageId ?? existing.localId,
          'sender_json': jsonEncode(messageSenderToMap(message.sender)),
          'text': message.text,
          'created_at_ms': platformInt64ToInt(message.createdAtMs),
          'server_order_key': platformInt64ToInt(message.messageSeq),
          'sender_type': message.senderType.name,
          'reply_to_message_id': message.replyTo?.messageId,
          'reply_to_preview_json': message.replyTo == null
              ? null
              : jsonEncode(_replyPreviewToMap(message.replyTo!)),
          'recalled_at_ms': platformInt64ToNullableInt(message.recalledAtMs),
          'mentioned_account_ids_json': jsonEncode(message.mentionedAccountIds),
          'mentioned_agent_ids_json': jsonEncode(message.mentionedAgentIds),
          'delivery_state': SocialMessageDeliveryState.sent.name,
          'client_seq': existing.clientSeq,
          'reactions_json': _reactionsToJson(message.reactions),
        },
        where: 'local_id = ?',
        whereArgs: <Object>[localId],
      );
    });
    return _loadMessageByLocalId(db, localId);
  }

  /// Update rail preview. [createdAtMs] advances [last_message_at_ms] only via
  /// `max(prev, createdAtMs)` — never invents wall clock, never regresses on
  /// stale/recall frames with an older message timestamp.
  Future<void> touchConversationPreview({
    required String conversationId,
    required String preview,
    required int createdAtMs,
    int? unreadCount,
    int? unreadMentionCount,
  }) async {
    final db = await _database();
    if (db == null) {
      return;
    }
    final existing = await db.query(
      'cached_social_conversations',
      columns: <String>['last_message_at_ms'],
      where: 'conversation_id = ?',
      whereArgs: <Object>[conversationId],
      limit: 1,
    );
    final prevAt = existing.isEmpty
        ? 0
        : (existing.first['last_message_at_ms'] as int? ?? 0);
    final incoming = createdAtMs > 0 ? createdAtMs : 0;
    final nextAt = incoming > 0
        ? (incoming > prevAt ? incoming : prevAt)
        : prevAt;
    final values = <String, Object?>{
      'last_message_preview': preview,
      'last_message_at_ms': nextAt,
    };
    if (unreadCount != null) {
      values['unread_count'] = unreadCount;
    }
    if (unreadMentionCount != null) {
      values['unread_mention_count'] = unreadMentionCount;
    }
    final updated = await db.update(
      'cached_social_conversations',
      values,
      where: 'conversation_id = ?',
      whereArgs: <Object>[conversationId],
    );
    // Unknown conversation: insert a minimal shell so multi-end inbox shows.
    if (updated == 0) {
      await db.insert(
        'cached_social_conversations',
        <String, Object?>{
          'conversation_id': conversationId,
          'kind': ConversationKind.group.name,
          'title': 'Conversation',
          'counterpart_json': null,
          'member_count': 0,
          'last_message_preview': preview,
          'last_message_at_ms': nextAt,
          'unread_count': unreadCount ?? 0,
          'unread_mention_count': unreadMentionCount ?? 0,
        },
        conflictAlgorithm: ConflictAlgorithm.replace,
      );
    }
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
    // Prefer match by server id, then by client_message_id (optimistic pending).
    var existingRows = await txn.query(
      'cached_social_messages',
      where: 'server_message_id = ?',
      whereArgs: <Object>[message.messageId],
      limit: 1,
    );
    if (existingRows.isEmpty) {
      existingRows = await txn.query(
        'cached_social_messages',
        where: 'client_message_id = ? OR local_id = ?',
        whereArgs: <Object>[message.messageId, message.messageId],
        limit: 1,
      );
    }
    if (existingRows.isNotEmpty) {
      final existing = _messageFromRow(existingRows.first);
      await txn.update(
        'cached_social_messages',
        <String, Object?>{
          'conversation_id': message.conversationId,
          'server_message_id': message.messageId,
          'client_message_id': existing.clientMessageId ?? message.messageId,
          'sender_json': jsonEncode(messageSenderToMap(message.sender)),
          'text': message.text,
          'created_at_ms': platformInt64ToInt(message.createdAtMs),
          'server_order_key': platformInt64ToInt(message.messageSeq),
          'sender_type': message.senderType.name,
          'reply_to_message_id': message.replyTo?.messageId,
          'reply_to_preview_json': message.replyTo == null
              ? null
              : jsonEncode(_replyPreviewToMap(message.replyTo!)),
          'recalled_at_ms': platformInt64ToNullableInt(message.recalledAtMs),
          'mentioned_account_ids_json': jsonEncode(message.mentionedAccountIds),
          'mentioned_agent_ids_json': jsonEncode(message.mentionedAgentIds),
          'delivery_state': SocialMessageDeliveryState.sent.name,
          'client_seq': existing.clientSeq,
          'reactions_json': _reactionsToJson(message.reactions),
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
      'client_message_id': message.messageId,
      'sender_json': jsonEncode(messageSenderToMap(message.sender)),
      'text': message.text,
      'created_at_ms': platformInt64ToInt(message.createdAtMs),
      'client_seq': nextClientSeq,
      'server_order_key': platformInt64ToInt(message.messageSeq),
      'sender_type': message.senderType.name,
      'reply_to_message_id': message.replyTo?.messageId,
      'reply_to_preview_json': message.replyTo == null
          ? null
          : jsonEncode(_replyPreviewToMap(message.replyTo!)),
      'recalled_at_ms': platformInt64ToNullableInt(message.recalledAtMs),
      'mentioned_account_ids_json': jsonEncode(message.mentionedAccountIds),
      'mentioned_agent_ids_json': jsonEncode(message.mentionedAgentIds),
      'delivery_state': SocialMessageDeliveryState.sent.name,
      'reactions_json': _reactionsToJson(message.reactions),
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
      lastMessageAtMs: platformInt64FromInt(row['last_message_at_ms']! as int),
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
      'last_message_at_ms': platformInt64ToInt(conversation.lastMessageAtMs),
      'unread_count': conversation.unreadCount,
      'unread_mention_count': conversation.unreadMentionCount,
    };
  }

  SocialChatMessage _messageFromRow(Map<String, Object?> row) {
    final localId = row['local_id']! as String;
    return SocialChatMessage(
      localId: localId,
      conversationId: row['conversation_id']! as String,
      sender: _messageSenderFromJson(row['sender_json'] as String?),
      text: row['text']! as String,
      createdAtMs: row['created_at_ms']! as int,
      clientSeq: row['client_seq']! as int,
      deliveryState: SocialMessageDeliveryState.values.byName(
        row['delivery_state']! as String,
      ),
      senderType: SenderType.values.byName(
        row['sender_type'] as String? ?? SenderType.user.name,
      ),
      serverMessageId: row['server_message_id'] as String?,
      clientMessageId: row['client_message_id'] as String? ?? localId,
      serverOrderKey: row['server_order_key'] as int?,
      replyTo: _replyPreviewFromJson(row['reply_to_preview_json'] as String?),
      recalledAtMs: row['recalled_at_ms'] as int?,
      mentionedAccountIds: _stringListFromJson(
        row['mentioned_account_ids_json'] as String?,
      ),
      mentionedAgentIds: _stringListFromJson(
        row['mentioned_agent_ids_json'] as String?,
      ),
      reactions: _reactionsFromJson(row['reactions_json'] as String?),
    );
  }

  Map<String, Object?> _messageToRow(SocialChatMessage message) {
    return <String, Object?>{
      'local_id': message.localId,
      'conversation_id': message.conversationId,
      'server_message_id': message.serverMessageId,
      'client_message_id': message.clientMessageId ?? message.localId,
      'sender_json': jsonEncode(messageSenderToMap(message.sender)),
      'text': message.text,
      'created_at_ms': message.createdAtMs,
      'client_seq': message.clientSeq,
      'server_order_key': message.serverOrderKey,
      'sender_type': message.senderType.name,
      'reply_to_message_id': message.replyTo?.messageId,
      'reply_to_preview_json': message.replyTo == null
          ? null
          : jsonEncode(_replyPreviewToMap(message.replyTo!)),
      'recalled_at_ms': message.recalledAtMs,
      'mentioned_account_ids_json': jsonEncode(message.mentionedAccountIds),
      'mentioned_agent_ids_json': jsonEncode(message.mentionedAgentIds),
      'delivery_state': message.deliveryState.name,
      'reactions_json': _reactionsToJson(message.reactions),
    };
  }

  List<String> _stringListFromJson(String? raw) {
    if (raw == null || raw.isEmpty) {
      return const <String>[];
    }
    try {
      final decoded = jsonDecode(raw);
      if (decoded is! List) {
        return const <String>[];
      }
      return decoded.map((value) => value as String).toList(growable: false);
    } catch (_) {
      return const <String>[];
    }
  }

  List<ReactionGroup> _reactionsFromJson(String? raw) {
    if (raw == null || raw.isEmpty || raw == '[]') {
      return const <ReactionGroup>[];
    }
    try {
      final list = jsonDecode(raw) as List<dynamic>;
      return list
          .map((item) {
            final map = item as Map<String, dynamic>;
            final actorsRaw = map['actors'] as List<dynamic>? ?? const [];
            return ReactionGroup(
              emoji: map['emoji'] as String? ?? '',
              count: (map['count'] as num?)?.toInt() ?? 0,
              reactedByMe: map['reacted_by_me'] as bool? ?? false,
              actors: actorsRaw
                  .map((a) {
                    final am = a as Map<String, dynamic>;
                    return ReactionActor(
                      actorId: am['actor_id'] as String? ?? '',
                      actorKind: am['actor_kind'] as String? ?? 'user',
                      displayName: am['display_name'] as String? ?? '',
                    );
                  })
                  .toList(growable: false),
            );
          })
          .where((g) => g.emoji.isNotEmpty)
          .toList(growable: false);
    } catch (_) {
      return const <ReactionGroup>[];
    }
  }

  String _reactionsToJson(List<ReactionGroup> reactions) {
    if (reactions.isEmpty) return '[]';
    return jsonEncode(
      reactions
          .map(
            (g) => <String, Object?>{
              'emoji': g.emoji,
              'count': g.count,
              'reacted_by_me': g.reactedByMe,
              'actors': g.actors
                  .map(
                    (a) => <String, Object?>{
                      'actor_id': a.actorId,
                      'actor_kind': a.actorKind,
                      'display_name': a.displayName,
                    },
                  )
                  .toList(growable: false),
            },
          )
          .toList(growable: false),
    );
  }

  /// Update reaction aggregate for a server message (inbound + local apply).
  Future<void> updateMessageReactions({
    required String conversationId,
    required String messageId,
    required List<ReactionGroup> reactions,
  }) async {
    final db = await _database();
    if (db == null) return;
    final mid = messageId.trim();
    if (mid.isEmpty) return;
    final updated = await db.update(
      'cached_social_messages',
      <String, Object?>{'reactions_json': _reactionsToJson(reactions)},
      where: 'conversation_id = ? AND server_message_id = ?',
      whereArgs: <Object>[conversationId, mid],
    );
    if (updated == 0) {
      // Match optimistic local rows by client_message_id as fallback.
      await db.update(
        'cached_social_messages',
        <String, Object?>{'reactions_json': _reactionsToJson(reactions)},
        where: 'conversation_id = ? AND client_message_id = ?',
        whereArgs: <Object>[conversationId, mid],
      );
    }
  }

  Map<String, Object?> _replyPreviewToMap(ChatMessageReplySummary reply) {
    return <String, Object?>{
      'message_id': reply.messageId,
      'sender': messageSenderToMap(reply.sender),
      'text': reply.text,
      'recalled_at_ms': platformInt64ToNullableInt(reply.recalledAtMs),
    };
  }

  ChatMessageReplySummary? _replyPreviewFromJson(String? raw) {
    if (raw == null || raw.isEmpty) {
      return null;
    }
    final map = jsonDecode(raw) as Map<String, Object?>;
    return ChatMessageReplySummary(
      messageId: map['message_id']! as String,
      sender: messageSenderFromMap(map['sender']! as Map<String, Object?>),
      text: map['text']! as String,
      recalledAtMs: map['recalled_at_ms'] == null
          ? null
          : platformInt64FromInt(map['recalled_at_ms']! as int),
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

  MessageSender _messageSenderFromJson(String? raw) {
    if (raw == null || raw.isEmpty) {
      return const MessageSender.account(
        accountId: '',
        minosId: '',
        displayName: '',
      );
    }
    return messageSenderFromMap(jsonDecode(raw) as Map<String, Object?>);
  }

  /// UUIDv4 used as wire `client_message_id` / local pending primary key.
  String _newClientMessageId() {
    final r = Random.secure();
    final bytes = List<int>.generate(16, (_) => r.nextInt(256));
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    String hex(int b) => b.toRadixString(16).padLeft(2, '0');
    final h = bytes.map(hex).join();
    return '${h.substring(0, 8)}-${h.substring(8, 12)}-'
        '${h.substring(12, 16)}-${h.substring(16, 20)}-${h.substring(20)}';
  }
}
