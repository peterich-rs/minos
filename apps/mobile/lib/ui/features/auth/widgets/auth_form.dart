import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:minos/ui/theme/theme.dart';

final _authFormValidationProvider = NotifierProvider.autoDispose
    .family<_AuthFormValidationController, AuthFormValidationState, String>(
      _AuthFormValidationController.new,
    );

class _AuthFormValidationController extends Notifier<AuthFormValidationState> {
  _AuthFormValidationController(String _);

  @override
  AuthFormValidationState build() => const AuthFormValidationState();

  void update(AuthFormValidationState next) {
    if (state == next) return;
    state = next;
  }
}

class AuthFormValidationState {
  const AuthFormValidationState({
    this.emailError,
    this.passwordError,
    this.confirmError,
  });

  final String? emailError;
  final String? passwordError;
  final String? confirmError;

  bool get isValid =>
      emailError == null && passwordError == null && confirmError == null;

  @override
  bool operator ==(Object other) {
    return other is AuthFormValidationState &&
        other.emailError == emailError &&
        other.passwordError == passwordError &&
        other.confirmError == confirmError;
  }

  @override
  int get hashCode => Object.hash(emailError, passwordError, confirmError);
}

/// Two-mode auth form: e-mail + password (+ confirm in register).
enum AuthMode { login, register }

class AuthForm extends ConsumerStatefulWidget {
  const AuthForm({
    super.key,
    required this.mode,
    required this.onModeChanged,
    required this.onSubmit,
    required this.inFlight,
  });

  final AuthMode mode;
  final ValueChanged<AuthMode> onModeChanged;
  final Future<void> Function(String email, String password) onSubmit;
  final bool inFlight;

  @override
  ConsumerState<AuthForm> createState() => _AuthFormState();
}

class _AuthFormState extends ConsumerState<AuthForm> {
  final _emailCtl = TextEditingController();
  final _passwordCtl = TextEditingController();
  final _confirmCtl = TextEditingController();
  late final String _formId = 'auth-form-${identityHashCode(this)}';

  static final _emailRe = RegExp(r'^[^\s@]+@[^\s@]+\.[^\s@]+$');

  @override
  void dispose() {
    _emailCtl.dispose();
    _passwordCtl.dispose();
    _confirmCtl.dispose();
    super.dispose();
  }

  bool _validate() {
    final email = _emailCtl.text.trim();
    final pwd = _passwordCtl.text;
    final next = AuthFormValidationState(
      emailError: _emailRe.hasMatch(email) ? null : '邮箱格式不正确',
      passwordError: pwd.length >= 8 ? null : '至少 8 个字符',
      confirmError: widget.mode == AuthMode.register
          ? (_confirmCtl.text == pwd ? null : '两次密码不一致')
          : null,
    );
    ref.read(_authFormValidationProvider(_formId).notifier).update(next);
    return next.isValid;
  }

  Future<void> _handleSubmit() async {
    if (!_validate()) return;
    await widget.onSubmit(_emailCtl.text.trim(), _passwordCtl.text);
  }

  @override
  Widget build(BuildContext context) {
    final validation = ref.watch(_authFormValidationProvider(_formId));
    final isRegister = widget.mode == AuthMode.register;
    final submitLabel = isRegister ? '注册' : '登录';
    final toggleLabel = isRegister ? '已有账号？登录' : '创建新账号';
    final colors = context.minosColors;
    final theme = Theme.of(context);
    final errorStyle = theme.textTheme.bodySmall?.copyWith(
      color: colors.danger,
    );

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: <Widget>[
        TextField(
          controller: _emailCtl,
          enabled: !widget.inFlight,
          keyboardType: TextInputType.emailAddress,
          autocorrect: false,
          textInputAction: TextInputAction.next,
          decoration: const InputDecoration(
            hintText: '邮箱',
            prefixIcon: Icon(Icons.mail_outline_rounded),
          ),
        ),
        if (validation.emailError != null)
          Padding(
            padding: const EdgeInsets.only(top: MinosSpacing.xs),
            child: Text(validation.emailError!, style: errorStyle),
          ),
        const SizedBox(height: MinosSpacing.md),
        TextField(
          controller: _passwordCtl,
          enabled: !widget.inFlight,
          obscureText: true,
          textInputAction: isRegister
              ? TextInputAction.next
              : TextInputAction.done,
          onSubmitted: widget.inFlight || isRegister
              ? null
              : (_) => _handleSubmit(),
          decoration: const InputDecoration(
            hintText: '密码',
            prefixIcon: Icon(Icons.lock_outline_rounded),
          ),
        ),
        if (validation.passwordError != null)
          Padding(
            padding: const EdgeInsets.only(top: MinosSpacing.xs),
            child: Text(validation.passwordError!, style: errorStyle),
          ),
        if (isRegister) ...<Widget>[
          const SizedBox(height: MinosSpacing.md),
          TextField(
            controller: _confirmCtl,
            enabled: !widget.inFlight,
            obscureText: true,
            textInputAction: TextInputAction.done,
            onSubmitted: widget.inFlight ? null : (_) => _handleSubmit(),
            decoration: const InputDecoration(
              hintText: '确认密码',
              prefixIcon: Icon(Icons.lock_outline_rounded),
            ),
          ),
          if (validation.confirmError != null)
            Padding(
              padding: const EdgeInsets.only(top: MinosSpacing.xs),
              child: Text(validation.confirmError!, style: errorStyle),
            ),
        ],
        const SizedBox(height: MinosSpacing.xl),
        FilledButton(
          onPressed: widget.inFlight ? null : _handleSubmit,
          child: widget.inFlight
              ? SizedBox(
                  width: 18,
                  height: 18,
                  child: CircularProgressIndicator(
                    strokeWidth: 2,
                    color: colors.textOnAccent,
                  ),
                )
              : Text(submitLabel),
        ),
        const SizedBox(height: MinosSpacing.sm),
        TextButton(
          onPressed: widget.inFlight
              ? null
              : () => widget.onModeChanged(
                  isRegister ? AuthMode.login : AuthMode.register,
                ),
          child: Text(toggleLabel),
        ),
      ],
    );
  }
}
