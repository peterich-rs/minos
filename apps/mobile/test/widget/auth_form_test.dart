import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shadcn_ui/shadcn_ui.dart';

import 'package:minos/presentation/widgets/auth/auth_form.dart';

/// Wraps [AuthForm] in the minimum [ShadApp]/[Scaffold] scaffolding the
/// shadcn_ui widgets require to look up [ShadTheme] / [Material] ancestors.
Widget _harness({
  required AuthMode mode,
  required ValueChanged<AuthMode> onModeChanged,
  required Future<void> Function(String, String) onSubmit,
  bool inFlight = false,
}) {
  return ShadApp(
    home: Scaffold(
      body: SafeArea(
        child: Padding(
          padding: const EdgeInsets.all(16),
          child: AuthForm(
            mode: mode,
            onModeChanged: onModeChanged,
            onSubmit: onSubmit,
            inFlight: inFlight,
          ),
        ),
      ),
    ),
  );
}

void main() {
  testWidgets('Empty submit shows validation errors and does not call onSubmit',
      (tester) async {
    var submits = 0;
    await tester.pumpWidget(_harness(
      mode: AuthMode.login,
      onModeChanged: (_) {},
      onSubmit: (_, _) async {
        submits += 1;
      },
    ));

    // Submit button is the only ShadButton in login mode.
    final submitFinder = find.widgetWithText(ShadButton, 'Log in');
    expect(submitFinder, findsOneWidget);
    // Button stays enabled even when fields are empty — validation runs on
    // tap, not as a precondition.
    expect((tester.widget(submitFinder) as ShadButton).enabled, isTrue);

    await tester.tap(submitFinder);
    await tester.pump();

    expect(find.text('Invalid email'), findsOneWidget);
    expect(find.text('Min 8 characters'), findsOneWidget);
    expect(submits, 0);
  });

  testWidgets('inFlight=true disables the submit button and shows a spinner',
      (tester) async {
    await tester.pumpWidget(_harness(
      mode: AuthMode.login,
      onModeChanged: (_) {},
      onSubmit: (_, _) async {},
      inFlight: true,
    ));

    final btn =
        tester.widget<ShadButton>(find.byType(ShadButton).first);
    expect(btn.enabled, isFalse);
    expect(find.byType(CircularProgressIndicator), findsOneWidget);
  });

  testWidgets('Mode toggle reveals the confirm-password field in register mode',
      (tester) async {
    AuthMode current = AuthMode.login;
    late StateSetter setter;
    await tester.pumpWidget(StatefulBuilder(
      builder: (context, setState) {
        setter = setState;
        return _harness(
          mode: current,
          onModeChanged: (m) => setter(() => current = m),
          onSubmit: (_, _) async {},
        );
      },
    ));

    // Login mode: 2 inputs (email + password).
    expect(find.byType(ShadInput), findsNWidgets(2));

    // Tap the toggle — it's a TextButton beneath the ShadButton.
    await tester.tap(find.text('Create account'));
    await tester.pump();

    // Register mode: 3 inputs (email + password + confirm).
    expect(find.byType(ShadInput), findsNWidgets(3));
    expect(find.widgetWithText(ShadButton, 'Register'), findsOneWidget);
  });

  testWidgets('Valid submit calls onSubmit with trimmed email + raw password',
      (tester) async {
    String? capturedEmail;
    String? capturedPassword;
    await tester.pumpWidget(_harness(
      mode: AuthMode.login,
      onModeChanged: (_) {},
      onSubmit: (e, p) async {
        capturedEmail = e;
        capturedPassword = p;
      },
    ));

    final inputs = find.byType(ShadInput);
    await tester.enterText(inputs.at(0), '  user@example.com  ');
    await tester.enterText(inputs.at(1), 'hunter2hunter2');

    await tester.tap(find.widgetWithText(ShadButton, 'Log in'));
    await tester.pumpAndSettle();

    expect(capturedEmail, 'user@example.com');
    expect(capturedPassword, 'hunter2hunter2');
  });

  testWidgets('Register-mode confirm mismatch surfaces an error', (tester) async {
    await tester.pumpWidget(_harness(
      mode: AuthMode.register,
      onModeChanged: (_) {},
      onSubmit: (_, _) async {},
    ));

    final inputs = find.byType(ShadInput);
    await tester.enterText(inputs.at(0), 'user@example.com');
    await tester.enterText(inputs.at(1), 'hunter2hunter2');
    await tester.enterText(inputs.at(2), 'something_else');

    await tester.tap(find.widgetWithText(ShadButton, 'Register'));
    await tester.pump();

    expect(find.text('Does not match'), findsOneWidget);
  });
}
