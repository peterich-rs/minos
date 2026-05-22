// GENERATED CODE - DO NOT MODIFY BY HAND

part of 'agent_editor_state.dart';

// **************************************************************************
// RiverpodGenerator
// **************************************************************************

// GENERATED CODE - DO NOT MODIFY BY HAND
// ignore_for_file: type=lint, type=warning

@ProviderFor(AgentEditorDraftController)
final agentEditorDraftControllerProvider = AgentEditorDraftControllerFamily._();

final class AgentEditorDraftControllerProvider
    extends $NotifierProvider<AgentEditorDraftController, AgentEditorState> {
  AgentEditorDraftControllerProvider._({
    required AgentEditorDraftControllerFamily super.from,
    required AgentProfileDraft super.argument,
  }) : super(
         retry: null,
         name: r'agentEditorDraftControllerProvider',
         isAutoDispose: true,
         dependencies: null,
         $allTransitiveDependencies: null,
       );

  @override
  String debugGetCreateSourceHash() => _$agentEditorDraftControllerHash();

  @override
  String toString() {
    return r'agentEditorDraftControllerProvider'
        ''
        '($argument)';
  }

  @$internal
  @override
  AgentEditorDraftController create() => AgentEditorDraftController();

  /// {@macro riverpod.override_with_value}
  Override overrideWithValue(AgentEditorState value) {
    return $ProviderOverride(
      origin: this,
      providerOverride: $SyncValueProvider<AgentEditorState>(value),
    );
  }

  @override
  bool operator ==(Object other) {
    return other is AgentEditorDraftControllerProvider &&
        other.argument == argument;
  }

  @override
  int get hashCode {
    return argument.hashCode;
  }
}

String _$agentEditorDraftControllerHash() =>
    r'c0bcf0c162bd9ed23898d94b551e828bb3fd099a';

final class AgentEditorDraftControllerFamily extends $Family
    with
        $ClassFamilyOverride<
          AgentEditorDraftController,
          AgentEditorState,
          AgentEditorState,
          AgentEditorState,
          AgentProfileDraft
        > {
  AgentEditorDraftControllerFamily._()
    : super(
        retry: null,
        name: r'agentEditorDraftControllerProvider',
        dependencies: null,
        $allTransitiveDependencies: null,
        isAutoDispose: true,
      );

  AgentEditorDraftControllerProvider call(AgentProfileDraft draft) =>
      AgentEditorDraftControllerProvider._(argument: draft, from: this);

  @override
  String toString() => r'agentEditorDraftControllerProvider';
}

abstract class _$AgentEditorDraftController
    extends $Notifier<AgentEditorState> {
  late final _$args = ref.$arg as AgentProfileDraft;
  AgentProfileDraft get draft => _$args;

  AgentEditorState build(AgentProfileDraft draft);
  @$mustCallSuper
  @override
  void runBuild() {
    final ref = this.ref as $Ref<AgentEditorState, AgentEditorState>;
    final element =
        ref.element
            as $ClassProviderElement<
              AnyNotifier<AgentEditorState, AgentEditorState>,
              AgentEditorState,
              Object?,
              Object?
            >;
    element.handleCreate(ref, () => build(_$args));
  }
}
