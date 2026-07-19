import 'package:flutter/material.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

class AgentQuestionRequestData {
  const AgentQuestionRequestData({
    required this.requestId,
    required this.questions,
  });

  final String requestId;
  final List<AgentQuestionData> questions;

  factory AgentQuestionRequestData.fromJson(Map<String, dynamic> json) {
    final properties = switch (json['properties']) {
      final Map<Object?, Object?> value => value.map(
        (key, value) => MapEntry('$key', value),
      ),
      _ => json,
    };
    final questions = switch (properties['questions']) {
      final List<Object?> value =>
        value
            .whereType<Map<Object?, Object?>>()
            .map(
              (item) => AgentQuestionData.fromJson(
                item.map((key, value) => MapEntry('$key', value)),
              ),
            )
            .toList(),
      _ => const <AgentQuestionData>[],
    };
    return AgentQuestionRequestData(
      requestId:
          properties['id'] as String? ??
          properties['requestID'] as String? ??
          '',
      questions: questions,
    );
  }
}

class AgentQuestionData {
  const AgentQuestionData({
    required this.header,
    required this.question,
    required this.options,
    required this.multiple,
    required this.custom,
  });

  final String header;
  final String question;
  final List<AgentQuestionOptionData> options;
  final bool multiple;
  final bool custom;

  factory AgentQuestionData.fromJson(Map<String, dynamic> json) {
    final options = switch (json['options']) {
      final List<Object?> value =>
        value
            .whereType<Map<Object?, Object?>>()
            .map(
              (item) => AgentQuestionOptionData.fromJson(
                item.map((key, value) => MapEntry('$key', value)),
              ),
            )
            .where((option) => option.label.isNotEmpty)
            .toList(),
      _ => const <AgentQuestionOptionData>[],
    };
    return AgentQuestionData(
      header: json['header'] as String? ?? '',
      question: json['question'] as String? ?? 'Question',
      options: options,
      multiple: json['multiple'] as bool? ?? false,
      custom: json['custom'] as bool? ?? false,
    );
  }
}

class AgentQuestionOptionData {
  const AgentQuestionOptionData({
    required this.label,
    required this.description,
  });

  final String label;
  final String description;

  factory AgentQuestionOptionData.fromJson(Map<String, dynamic> json) {
    return AgentQuestionOptionData(
      label: json['label'] as String? ?? json['value'] as String? ?? '',
      description: json['description'] as String? ?? '',
    );
  }
}

Future<List<List<String>>?> showAgentQuestionSheet(
  BuildContext context, {
  required AgentQuestionRequestData request,
}) {
  return showModalBottomSheet<List<List<String>>>(
    context: context,
    isScrollControlled: true,
    isDismissible: false,
    enableDrag: false,
    backgroundColor: Colors.transparent,
    builder: (_) => _AgentQuestionSheet(request: request),
  );
}

class _AgentQuestionSheet extends StatefulWidget {
  const _AgentQuestionSheet({required this.request});

  final AgentQuestionRequestData request;

  @override
  State<_AgentQuestionSheet> createState() => _AgentQuestionSheetState();
}

class _AgentQuestionSheetState extends State<_AgentQuestionSheet> {
  late final List<Set<String>> _selected = List.generate(
    widget.request.questions.length,
    (_) => <String>{},
  );
  late final List<TextEditingController> _customControllers = List.generate(
    widget.request.questions.length,
    (_) => TextEditingController(),
  );

  @override
  void dispose() {
    for (final controller in _customControllers) {
      controller.dispose();
    }
    super.dispose();
  }

  bool get _canSubmit {
    for (var i = 0; i < widget.request.questions.length; i++) {
      if (_answersFor(i).isEmpty) return false;
    }
    return widget.request.questions.isNotEmpty;
  }

  List<String> _answersFor(int index) {
    final custom = _customControllers[index].text.trim();
    if (custom.isNotEmpty) return <String>[custom];
    return _selected[index].toList(growable: false);
  }

  List<List<String>> _answers() {
    return List.generate(widget.request.questions.length, _answersFor);
  }

  @override
  Widget build(BuildContext context) {
    final theme = ShadTheme.of(context);
    return SafeArea(
      child: Container(
        constraints: BoxConstraints(
          maxHeight: MediaQuery.sizeOf(context).height * 0.86,
        ),
        decoration: BoxDecoration(
          color: theme.colorScheme.background,
          borderRadius: const BorderRadius.vertical(top: Radius.circular(16)),
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(20, 16, 20, 12),
              child: Row(
                children: [
                  const Icon(LucideIcons.messageCircle, size: 22),
                  const SizedBox(width: 10),
                  Expanded(child: Text('Agent 提问', style: theme.textTheme.h4)),
                ],
              ),
            ),
            Flexible(
              child: ListView.separated(
                padding: const EdgeInsets.fromLTRB(20, 0, 20, 16),
                itemCount: widget.request.questions.length,
                separatorBuilder: (_, _) => const SizedBox(height: 16),
                itemBuilder: (context, index) {
                  return _QuestionBlock(
                    question: widget.request.questions[index],
                    selected: _selected[index],
                    customController: _customControllers[index],
                    onChanged: () => setState(() {}),
                  );
                },
              ),
            ),
            Padding(
              padding: const EdgeInsets.fromLTRB(20, 0, 20, 20),
              child: SizedBox(
                width: double.infinity,
                child: ShadButton(
                  onPressed: _canSubmit
                      ? () => Navigator.of(context).pop(_answers())
                      : null,
                  child: const Text('提交答案'),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _QuestionBlock extends StatelessWidget {
  const _QuestionBlock({
    required this.question,
    required this.selected,
    required this.customController,
    required this.onChanged,
  });

  final AgentQuestionData question;
  final Set<String> selected;
  final TextEditingController customController;
  final VoidCallback onChanged;

  @override
  Widget build(BuildContext context) {
    final theme = ShadTheme.of(context);
    final title = question.header.isEmpty ? question.question : question.header;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(title, style: theme.textTheme.large),
        if (question.header.isNotEmpty) ...[
          const SizedBox(height: 4),
          Text(question.question, style: theme.textTheme.muted),
        ],
        const SizedBox(height: 10),
        if (question.multiple)
          for (final option in question.options)
            CheckboxListTile(
              contentPadding: EdgeInsets.zero,
              value: selected.contains(option.label),
              title: Text(option.label),
              subtitle: option.description.isEmpty
                  ? null
                  : Text(option.description),
              onChanged: (checked) {
                if (checked == true) {
                  selected.add(option.label);
                } else {
                  selected.remove(option.label);
                }
                customController.clear();
                onChanged();
              },
            )
        else
          RadioGroup<String>(
            groupValue: selected.isEmpty ? null : selected.first,
            onChanged: (value) {
              selected.clear();
              if (value != null) {
                selected.add(value);
              }
              customController.clear();
              onChanged();
            },
            child: Column(
              children: [
                for (final option in question.options)
                  RadioListTile<String>(
                    contentPadding: EdgeInsets.zero,
                    value: option.label,
                    title: Text(option.label),
                    subtitle: option.description.isEmpty
                        ? null
                        : Text(option.description),
                  ),
              ],
            ),
          ),
        if (question.custom || question.options.isEmpty) ...[
          const SizedBox(height: 8),
          TextField(
            controller: customController,
            minLines: 1,
            maxLines: 4,
            decoration: const InputDecoration(
              labelText: '自定义答案',
              border: OutlineInputBorder(),
            ),
            onChanged: (_) {
              if (customController.text.trim().isNotEmpty) {
                selected.clear();
              }
              onChanged();
            },
          ),
        ],
      ],
    );
  }
}
