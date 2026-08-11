// GENERATED CODE - DO NOT MODIFY BY HAND

part of 'social_chat_view_model.dart';

// **************************************************************************
// RiverpodGenerator
// **************************************************************************

// GENERATED CODE - DO NOT MODIFY BY HAND
// ignore_for_file: type=lint, type=warning
/// Feature-level ViewModel for the open conversation surface.
///
/// UI should prefer this aggregation over watching multiple fine-grained
/// providers: timeline state, reply draft target, and intentful actions.

@ProviderFor(SocialChatViewModel)
final socialChatViewModelProvider = SocialChatViewModelFamily._();

/// Feature-level ViewModel for the open conversation surface.
///
/// UI should prefer this aggregation over watching multiple fine-grained
/// providers: timeline state, reply draft target, and intentful actions.
final class SocialChatViewModelProvider
    extends $NotifierProvider<SocialChatViewModel, SocialConversationState> {
  /// Feature-level ViewModel for the open conversation surface.
  ///
  /// UI should prefer this aggregation over watching multiple fine-grained
  /// providers: timeline state, reply draft target, and intentful actions.
  SocialChatViewModelProvider._({
    required SocialChatViewModelFamily super.from,
    required String super.argument,
  }) : super(
         retry: null,
         name: r'socialChatViewModelProvider',
         isAutoDispose: true,
         dependencies: null,
         $allTransitiveDependencies: null,
       );

  @override
  String debugGetCreateSourceHash() => _$socialChatViewModelHash();

  @override
  String toString() {
    return r'socialChatViewModelProvider'
        ''
        '($argument)';
  }

  @$internal
  @override
  SocialChatViewModel create() => SocialChatViewModel();

  /// {@macro riverpod.override_with_value}
  Override overrideWithValue(SocialConversationState value) {
    return $ProviderOverride(
      origin: this,
      providerOverride: $SyncValueProvider<SocialConversationState>(value),
    );
  }

  @override
  bool operator ==(Object other) {
    return other is SocialChatViewModelProvider && other.argument == argument;
  }

  @override
  int get hashCode {
    return argument.hashCode;
  }
}

String _$socialChatViewModelHash() =>
    r'6c79ec19cc964c4625ee37039dbc6d8eedd5621d';

/// Feature-level ViewModel for the open conversation surface.
///
/// UI should prefer this aggregation over watching multiple fine-grained
/// providers: timeline state, reply draft target, and intentful actions.

final class SocialChatViewModelFamily extends $Family
    with
        $ClassFamilyOverride<
          SocialChatViewModel,
          SocialConversationState,
          SocialConversationState,
          SocialConversationState,
          String
        > {
  SocialChatViewModelFamily._()
    : super(
        retry: null,
        name: r'socialChatViewModelProvider',
        dependencies: null,
        $allTransitiveDependencies: null,
        isAutoDispose: true,
      );

  /// Feature-level ViewModel for the open conversation surface.
  ///
  /// UI should prefer this aggregation over watching multiple fine-grained
  /// providers: timeline state, reply draft target, and intentful actions.

  SocialChatViewModelProvider call(String conversationId) =>
      SocialChatViewModelProvider._(argument: conversationId, from: this);

  @override
  String toString() => r'socialChatViewModelProvider';
}

/// Feature-level ViewModel for the open conversation surface.
///
/// UI should prefer this aggregation over watching multiple fine-grained
/// providers: timeline state, reply draft target, and intentful actions.

abstract class _$SocialChatViewModel
    extends $Notifier<SocialConversationState> {
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
