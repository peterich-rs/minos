import 'package:minos/domain/agent_profile.dart';
import 'package:minos/src/rust/api/minos.dart';
import 'package:riverpod_annotation/riverpod_annotation.dart';

part 'agent_editor_state.g.dart';

@Riverpod(keepAlive: false)
class AgentEditorDraftController extends _$AgentEditorDraftController {
  @override
  AgentEditorState build(AgentProfileDraft draft) {
    return AgentEditorState(draft: draft);
  }

  void updateName(String name) {
    state = state.copyWith(draft: state.draft.copyWith(name: name));
  }

  void updateDescription(String description) {
    state = state.copyWith(
      draft: state.draft.copyWith(description: description),
    );
  }

  void setHost({
    required String? hostDeviceId,
    required String? hostDisplayName,
  }) {
    final draft = state.draft;
    final workspacePath = hostDeviceId == draft.hostDeviceId
        ? draft.workspacePath
        : null;
    state = state.copyWith(
      draft: AgentProfileDraft(
        name: draft.name,
        description: draft.description,
        runtimeAgent: draft.runtimeAgent,
        model: draft.model,
        reasoningEffort: draft.reasoningEffort,
        environmentVariables: draft.environmentVariables,
        hostDeviceId: hostDeviceId,
        hostDisplayName: hostDisplayName,
        workspacePath: workspacePath,
      ),
    );
  }

  void setWorkspacePath(String? workspacePath) {
    final draft = state.draft;
    state = state.copyWith(
      draft: AgentProfileDraft(
        name: draft.name,
        description: draft.description,
        runtimeAgent: draft.runtimeAgent,
        model: draft.model,
        reasoningEffort: draft.reasoningEffort,
        environmentVariables: draft.environmentVariables,
        hostDeviceId: draft.hostDeviceId,
        hostDisplayName: draft.hostDisplayName,
        workspacePath: workspacePath,
      ),
    );
  }

  void setRuntime(AgentName runtimeAgent, {required String defaultModel}) {
    state = state.copyWith(
      draft: state.draft.copyWith(
        runtimeAgent: runtimeAgent,
        model: defaultModel,
      ),
    );
  }

  void setModel(String model) {
    state = state.copyWith(draft: state.draft.copyWith(model: model));
  }

  void setReasoning(AgentReasoningEffort reasoningEffort) {
    state = state.copyWith(
      draft: state.draft.copyWith(reasoningEffort: reasoningEffort),
    );
  }

  void toggleAdvanced() {
    state = state.copyWith(showAdvanced: !state.showAdvanced);
  }

  void updateEnvironmentKey(int index, String key) {
    final variables = List<AgentEnvironmentVariable>.of(
      state.draft.environmentVariables,
    );
    if (index < 0 || index >= variables.length) return;
    variables[index] = variables[index].copyWith(key: key);
    state = state.copyWith(
      draft: state.draft.copyWith(environmentVariables: variables),
    );
  }

  void updateEnvironmentValue(int index, String value) {
    final variables = List<AgentEnvironmentVariable>.of(
      state.draft.environmentVariables,
    );
    if (index < 0 || index >= variables.length) return;
    variables[index] = variables[index].copyWith(value: value);
    state = state.copyWith(
      draft: state.draft.copyWith(environmentVariables: variables),
    );
  }

  void removeEnvironmentVariable(int index) {
    final variables = List<AgentEnvironmentVariable>.of(
      state.draft.environmentVariables,
    );
    if (index < 0 || index >= variables.length) return;
    variables.removeAt(index);
    state = state.copyWith(
      draft: state.draft.copyWith(environmentVariables: variables),
    );
  }

  void addEnvironmentVariable() {
    state = state.copyWith(
      draft: state.draft.copyWith(
        environmentVariables: <AgentEnvironmentVariable>[
          ...state.draft.environmentVariables,
          const AgentEnvironmentVariable(key: '', value: ''),
        ],
      ),
    );
  }
}

class AgentEditorState {
  const AgentEditorState({required this.draft, this.showAdvanced = false});

  final AgentProfileDraft draft;
  final bool showAdvanced;

  bool get canSave => draft.name.trim().isNotEmpty;

  AgentEditorState copyWith({AgentProfileDraft? draft, bool? showAdvanced}) {
    return AgentEditorState(
      draft: draft ?? this.draft,
      showAdvanced: showAdvanced ?? this.showAdvanced,
    );
  }
}
