import 'package:flutter/material.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

/// Two-mode auth form: e-mail + password (+ confirm in register). The form
/// owns its own controllers and inline validation, but delegates the
/// "actually call register/login" decision to the parent via [onSubmit].
///
/// The parent owns the in-flight flag so it can survive an orientation
/// change and so a single inline spinner doesn't have to know about the
/// network. While [inFlight] is `true`, all interactive surfaces are
/// disabled and the submit button shows a [CircularProgressIndicator].
enum AuthMode { login, register }

class AuthForm extends StatefulWidget {
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
  State<AuthForm> createState() => _AuthFormState();
}

class _AuthFormState extends State<AuthForm> {
  final _emailCtl = TextEditingController();
  final _passwordCtl = TextEditingController();
  final _confirmCtl = TextEditingController();
  String? _emailErr;
  String? _passwordErr;
  String? _confirmErr;

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
    setState(() {
      _emailErr = _emailRe.hasMatch(email) ? null : 'Invalid email';
      _passwordErr = pwd.length >= 8 ? null : 'Min 8 characters';
      if (widget.mode == AuthMode.register) {
        _confirmErr = _confirmCtl.text == pwd ? null : 'Does not match';
      } else {
        _confirmErr = null;
      }
    });
    return _emailErr == null && _passwordErr == null && _confirmErr == null;
  }

  Future<void> _handleSubmit() async {
    if (!_validate()) return;
    await widget.onSubmit(_emailCtl.text.trim(), _passwordCtl.text);
  }

  @override
  Widget build(BuildContext context) {
    final isRegister = widget.mode == AuthMode.register;
    final submitLabel = isRegister ? 'Register' : 'Log in';
    final toggleLabel =
        isRegister ? 'Have an account? Log in' : 'Create account';
    final errorStyle = TextStyle(
      color: Theme.of(context).colorScheme.error,
      fontSize: 12,
    );

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: <Widget>[
        ShadInput(
          controller: _emailCtl,
          placeholder: const Text('Email'),
          keyboardType: TextInputType.emailAddress,
          autocorrect: false,
          enabled: !widget.inFlight,
        ),
        if (_emailErr != null)
          Padding(
            padding: const EdgeInsets.only(top: 4),
            child: Text(_emailErr!, style: errorStyle),
          ),
        const SizedBox(height: 12),
        ShadInput(
          controller: _passwordCtl,
          placeholder: const Text('Password'),
          obscureText: true,
          enabled: !widget.inFlight,
        ),
        if (_passwordErr != null)
          Padding(
            padding: const EdgeInsets.only(top: 4),
            child: Text(_passwordErr!, style: errorStyle),
          ),
        if (isRegister) ...<Widget>[
          const SizedBox(height: 12),
          ShadInput(
            controller: _confirmCtl,
            placeholder: const Text('Confirm password'),
            obscureText: true,
            enabled: !widget.inFlight,
          ),
          if (_confirmErr != null)
            Padding(
              padding: const EdgeInsets.only(top: 4),
              child: Text(_confirmErr!, style: errorStyle),
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
              : () => widget.onModeChanged(
                    isRegister ? AuthMode.login : AuthMode.register,
                  ),
          child: Text(toggleLabel),
        ),
      ],
    );
  }
}
