// GENERATED CODE - DO NOT MODIFY BY HAND

part of 'social_conversation_notifier.dart';

// **************************************************************************
// RiverpodGenerator
// **************************************************************************

// GENERATED CODE - DO NOT MODIFY BY HAND
// ignore_for_file: type=lint, type=warning

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
    r'32e4fffbaa9f33065cfea6eff7bed4b020b94eeb';

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
