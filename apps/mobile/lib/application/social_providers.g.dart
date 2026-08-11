// GENERATED CODE - DO NOT MODIFY BY HAND

part of 'social_providers.dart';

// **************************************************************************
// RiverpodGenerator
// **************************************************************************

// GENERATED CODE - DO NOT MODIFY BY HAND
// ignore_for_file: type=lint, type=warning

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

@ProviderFor(SocialConversation)
final socialConversationProvider = SocialConversationFamily._();

final class SocialConversationProvider
    extends $NotifierProvider<SocialConversation, SocialConversationState> {
  SocialConversationProvider._({
    required SocialConversationFamily super.from,
    required String super.argument,
  }) : super(
         retry: null,
         name: r'socialConversationProvider',
         isAutoDispose: true,
         dependencies: null,
         $allTransitiveDependencies: null,
       );

  @override
  String debugGetCreateSourceHash() => _$socialConversationHash();

  @override
  String toString() {
    return r'socialConversationProvider'
        ''
        '($argument)';
  }

  @$internal
  @override
  SocialConversation create() => SocialConversation();

  /// {@macro riverpod.override_with_value}
  Override overrideWithValue(SocialConversationState value) {
    return $ProviderOverride(
      origin: this,
      providerOverride: $SyncValueProvider<SocialConversationState>(value),
    );
  }

  @override
  bool operator ==(Object other) {
    return other is SocialConversationProvider && other.argument == argument;
  }

  @override
  int get hashCode {
    return argument.hashCode;
  }
}

String _$socialConversationHash() =>
    r'875571911e9dcdcbed00d97cbfc8d6ba60755dd5';

final class SocialConversationFamily extends $Family
    with
        $ClassFamilyOverride<
          SocialConversation,
          SocialConversationState,
          SocialConversationState,
          SocialConversationState,
          String
        > {
  SocialConversationFamily._()
    : super(
        retry: null,
        name: r'socialConversationProvider',
        dependencies: null,
        $allTransitiveDependencies: null,
        isAutoDispose: true,
      );

  SocialConversationProvider call(String conversationId) =>
      SocialConversationProvider._(argument: conversationId, from: this);

  @override
  String toString() => r'socialConversationProvider';
}

abstract class _$SocialConversation extends $Notifier<SocialConversationState> {
  late final _$args = ref.$arg as String;
  String get conversationId => _$args;

  SocialConversationState build(String conversationId);
  @$mustCallSuper
  @override
  void runBuild() {
    final ref =
        this.ref as $Ref<SocialConversationState, SocialConversationState>;
    final element =
        ref.element
            as $ClassProviderElement<
              AnyNotifier<SocialConversationState, SocialConversationState>,
              SocialConversationState,
              Object?,
              Object?
            >;
    element.handleCreate(ref, () => build(_$args));
  }
}
