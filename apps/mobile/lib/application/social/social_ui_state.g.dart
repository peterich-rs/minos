// GENERATED CODE - DO NOT MODIFY BY HAND

part of 'social_ui_state.dart';

// **************************************************************************
// RiverpodGenerator
// **************************************************************************

// GENERATED CODE - DO NOT MODIFY BY HAND
// ignore_for_file: type=lint, type=warning
/// Currently open social chat conversation (focused for unread / markRead).
/// Distinct from "has timeline window" (provider alive with messages).

@ProviderFor(FocusedSocialConversationId)
final focusedSocialConversationIdProvider =
    FocusedSocialConversationIdProvider._();

/// Currently open social chat conversation (focused for unread / markRead).
/// Distinct from "has timeline window" (provider alive with messages).
final class FocusedSocialConversationIdProvider
    extends $NotifierProvider<FocusedSocialConversationId, String?> {
  /// Currently open social chat conversation (focused for unread / markRead).
  /// Distinct from "has timeline window" (provider alive with messages).
  FocusedSocialConversationIdProvider._()
    : super(
        from: null,
        argument: null,
        retry: null,
        name: r'focusedSocialConversationIdProvider',
        isAutoDispose: false,
        dependencies: null,
        $allTransitiveDependencies: null,
      );

  @override
  String debugGetCreateSourceHash() => _$focusedSocialConversationIdHash();

  @$internal
  @override
  FocusedSocialConversationId create() => FocusedSocialConversationId();

  /// {@macro riverpod.override_with_value}
  Override overrideWithValue(String? value) {
    return $ProviderOverride(
      origin: this,
      providerOverride: $SyncValueProvider<String?>(value),
    );
  }
}

String _$focusedSocialConversationIdHash() =>
    r'75eef236b7ed3de7c8d8be65f688b8084496bc7e';

/// Currently open social chat conversation (focused for unread / markRead).
/// Distinct from "has timeline window" (provider alive with messages).

abstract class _$FocusedSocialConversationId extends $Notifier<String?> {
  String? build();
  @$mustCallSuper
  @override
  void runBuild() {
    final ref = this.ref as $Ref<String?, String?>;
    final element =
        ref.element
            as $ClassProviderElement<
              AnyNotifier<String?, String?>,
              String?,
              Object?,
              Object?
            >;
    element.handleCreate(ref, build);
  }
}

@ProviderFor(SubscriptionLimitNoticeController)
final subscriptionLimitNoticeControllerProvider =
    SubscriptionLimitNoticeControllerProvider._();

final class SubscriptionLimitNoticeControllerProvider
    extends
        $NotifierProvider<
          SubscriptionLimitNoticeController,
          SubscriptionLimitNotice?
        > {
  SubscriptionLimitNoticeControllerProvider._()
    : super(
        from: null,
        argument: null,
        retry: null,
        name: r'subscriptionLimitNoticeControllerProvider',
        isAutoDispose: false,
        dependencies: null,
        $allTransitiveDependencies: null,
      );

  @override
  String debugGetCreateSourceHash() =>
      _$subscriptionLimitNoticeControllerHash();

  @$internal
  @override
  SubscriptionLimitNoticeController create() =>
      SubscriptionLimitNoticeController();

  /// {@macro riverpod.override_with_value}
  Override overrideWithValue(SubscriptionLimitNotice? value) {
    return $ProviderOverride(
      origin: this,
      providerOverride: $SyncValueProvider<SubscriptionLimitNotice?>(value),
    );
  }
}

String _$subscriptionLimitNoticeControllerHash() =>
    r'8c01829224dd30395a2926e12d8cdc30cd84298c';

abstract class _$SubscriptionLimitNoticeController
    extends $Notifier<SubscriptionLimitNotice?> {
  SubscriptionLimitNotice? build();
  @$mustCallSuper
  @override
  void runBuild() {
    final ref =
        this.ref as $Ref<SubscriptionLimitNotice?, SubscriptionLimitNotice?>;
    final element =
        ref.element
            as $ClassProviderElement<
              AnyNotifier<SubscriptionLimitNotice?, SubscriptionLimitNotice?>,
              SubscriptionLimitNotice?,
              Object?,
              Object?
            >;
    element.handleCreate(ref, build);
  }
}

@ProviderFor(SocialReplyDraft)
final socialReplyDraftProvider = SocialReplyDraftFamily._();

final class SocialReplyDraftProvider
    extends $NotifierProvider<SocialReplyDraft, String?> {
  SocialReplyDraftProvider._({
    required SocialReplyDraftFamily super.from,
    required String super.argument,
  }) : super(
         retry: null,
         name: r'socialReplyDraftProvider',
         isAutoDispose: true,
         dependencies: null,
         $allTransitiveDependencies: null,
       );

  @override
  String debugGetCreateSourceHash() => _$socialReplyDraftHash();

  @override
  String toString() {
    return r'socialReplyDraftProvider'
        ''
        '($argument)';
  }

  @$internal
  @override
  SocialReplyDraft create() => SocialReplyDraft();

  /// {@macro riverpod.override_with_value}
  Override overrideWithValue(String? value) {
    return $ProviderOverride(
      origin: this,
      providerOverride: $SyncValueProvider<String?>(value),
    );
  }

  @override
  bool operator ==(Object other) {
    return other is SocialReplyDraftProvider && other.argument == argument;
  }

  @override
  int get hashCode {
    return argument.hashCode;
  }
}

String _$socialReplyDraftHash() => r'dd4fb184db1a7a9be3ebc79f00b15305a656fd52';

final class SocialReplyDraftFamily extends $Family
    with
        $ClassFamilyOverride<
          SocialReplyDraft,
          String?,
          String?,
          String?,
          String
        > {
  SocialReplyDraftFamily._()
    : super(
        retry: null,
        name: r'socialReplyDraftProvider',
        dependencies: null,
        $allTransitiveDependencies: null,
        isAutoDispose: true,
      );

  SocialReplyDraftProvider call(String conversationId) =>
      SocialReplyDraftProvider._(argument: conversationId, from: this);

  @override
  String toString() => r'socialReplyDraftProvider';
}

abstract class _$SocialReplyDraft extends $Notifier<String?> {
  late final _$args = ref.$arg as String;
  String get conversationId => _$args;

  String? build(String conversationId);
  @$mustCallSuper
  @override
  void runBuild() {
    final ref = this.ref as $Ref<String?, String?>;
    final element =
        ref.element
            as $ClassProviderElement<
              AnyNotifier<String?, String?>,
              String?,
              Object?,
              Object?
            >;
    element.handleCreate(ref, () => build(_$args));
  }
}
