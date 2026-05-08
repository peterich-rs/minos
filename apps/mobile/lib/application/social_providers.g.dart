// GENERATED CODE - DO NOT MODIFY BY HAND

part of 'social_providers.dart';

// **************************************************************************
// RiverpodGenerator
// **************************************************************************

// GENERATED CODE - DO NOT MODIFY BY HAND
// ignore_for_file: type=lint, type=warning

@ProviderFor(SocialSearchQuery)
final socialSearchQueryProvider = SocialSearchQueryProvider._();

final class SocialSearchQueryProvider
    extends $NotifierProvider<SocialSearchQuery, String> {
  SocialSearchQueryProvider._()
    : super(
        from: null,
        argument: null,
        retry: null,
        name: r'socialSearchQueryProvider',
        isAutoDispose: true,
        dependencies: null,
        $allTransitiveDependencies: null,
      );

  @override
  String debugGetCreateSourceHash() => _$socialSearchQueryHash();

  @$internal
  @override
  SocialSearchQuery create() => SocialSearchQuery();

  /// {@macro riverpod.override_with_value}
  Override overrideWithValue(String value) {
    return $ProviderOverride(
      origin: this,
      providerOverride: $SyncValueProvider<String>(value),
    );
  }
}

String _$socialSearchQueryHash() => r'8d3b2514681408de97b829d3eb39924c580818ac';

abstract class _$SocialSearchQuery extends $Notifier<String> {
  String build();
  @$mustCallSuper
  @override
  void runBuild() {
    final ref = this.ref as $Ref<String, String>;
    final element =
        ref.element
            as $ClassProviderElement<
              AnyNotifier<String, String>,
              String,
              Object?,
              Object?
            >;
    element.handleCreate(ref, build);
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
    r'59692406893f26c1488b009a0f38c511ec768972';

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
