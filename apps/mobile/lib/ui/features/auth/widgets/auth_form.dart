import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

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

/// Two-mode auth form: e-mail + password (+ confirm in register). The form
/// owns its own controllers and inline validation, but delegates the
/// "actually call register/login" decision to the parent via [onSubmit].
///
/// The parent owns the in-flight flag so it can survive an orientation
/// change and so a single inline spinner doesn't have to know about the
/// network. While [inFlight] is `true`, all interactive surfaces are
/// disabled and the submit button shows a [CircularProgressIndicator].
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

  /// Called only after local validation passes. The parent is expected to
  /// flip [inFlight] to `true` before awaiting the network call and back to
  /// `false` in a `finally` block.
  final Future<void> Function(String email, String password) onSubmit;

  /// Disables fields + submit button and swaps the submit label for a
  /// spinner. Owned by the parent so it survives a rebuild of this widget.
  final bool inFlight;

  @override
  ConsumerState<AuthForm> createState() => _AuthFormState();
}

class _AuthFormState extends ConsumerState<AuthForm> {
  final _emailCtl = TextEditingController();
  final _passwordCtl = TextEditingController();
  final _confirmCtl = TextEditingController();
  late final String _formId = 'auth-form-${identityHashCode(this)}';

  // Permissive: the canonical check happens server-side. We just rule out
  // obvious typos so the user doesn't burn an RPC round-trip.
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
      emailError: _emailRe.hasMatch(email) ? null : 'Invalid email',
      passwordError: pwd.length >= 8 ? null : 'Min 8 characters',
      confirmError: widget.mode == AuthMode.register
          ? (_confirmCtl.text == pwd ? null : 'Does not match')
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
    final isRegister = widget.mode == .register;
    final submitLabel = isRegister ? 'Register' : 'Log in';
    final toggleLabel = isRegister
        ? 'Have an account? Log in'
        : 'Create account';
    final errorStyle = TextStyle(
      color: Theme.of(context).colorScheme.error,
      fontSize: 12,
    );

    return Column(
      crossAxisAlignment: .stretch,
      children: <Widget>[
        ShadInput(
          controller: _emailCtl,
          placeholder: const Text('Email'),
          keyboardType: .emailAddress,
          autocorrect: false,
          enabled: !widget.inFlight,
        ),
        if (validation.emailError != null)
          Padding(
            padding: const EdgeInsets.only(top: 4),
            child: Text(validation.emailError!, style: errorStyle),
          ),
        const SizedBox(height: 12),
        ShadInput(
          controller: _passwordCtl,
          placeholder: const Text('Password'),
          obscureText: true,
          enabled: !widget.inFlight,
        ),
        if (validation.passwordError != null)
          Padding(
            padding: const EdgeInsets.only(top: 4),
            child: Text(validation.passwordError!, style: errorStyle),
          ),
        if (isRegister) ...<Widget>[
          const SizedBox(height: 12),
          ShadInput(
            controller: _confirmCtl,
            placeholder: const Text('Confirm password'),
            obscureText: true,
            enabled: !widget.inFlight,
          ),
          if (validation.confirmError != null)
            Padding(
              padding: const EdgeInsets.only(top: 4),
              child: Text(validation.confirmError!, style: errorStyle),
            ),
        ],
        const SizedBox(height: 20),
        ShadButton(
          enabled: !widget.inFlight,
          onPressed: widget.inFlight ? null : _handleSubmit,
          child: widget.inFlight
              ? const SizedBox(
                  width: 16,
                  height: 16,
                  child: CircularProgressIndicator(strokeWidth: 2),
                )
              : Text(submitLabel),
        ),
        const SizedBox(height: 8),
        TextButton(
          onPressed: widget.inFlight
              ? null
              : () => widget.onModeChanged(isRegister ? .login : .register),
          child: Text(toggleLabel),
        ),
      ],
    );
  }
}
