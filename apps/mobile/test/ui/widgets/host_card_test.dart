import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:minos/ui/features/hosts/widgets/host_card.dart';
import 'package:minos/ui/theme/theme.dart';

void main() {
  Widget wrap(Widget child) {
    return MaterialApp(
      theme: MinosTheme.light(),
      home: Scaffold(body: child),
    );
  }

  testWidgets('HostCard shows display name and online status', (tester) async {
    var tapped = false;
    await tester.pumpWidget(
      wrap(
        HostCard(
          displayName: 'Studio Mac',
          online: true,
          selected: false,
          onTap: () => tapped = true,
        ),
      ),
    );

    expect(find.text('Studio Mac'), findsOneWidget);
    expect(find.textContaining('在线'), findsOneWidget);

    await tester.tap(find.byType(HostCard));
    await tester.pump();
    expect(tapped, isTrue);
  });

  testWidgets('HostCard offline shows offline label', (tester) async {
    await tester.pumpWidget(
      wrap(
        HostCard(
          displayName: 'Laptop',
          online: false,
          selected: true,
          onTap: () {},
        ),
      ),
    );

    expect(find.text('Laptop'), findsOneWidget);
    expect(find.textContaining('离线'), findsOneWidget);
  });
}
