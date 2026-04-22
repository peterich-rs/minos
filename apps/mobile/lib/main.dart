import 'package:flutter/material.dart';

void main() {
  runApp(const MinosBootShell());
}

class MinosBootShell extends StatelessWidget {
  const MinosBootShell({super.key});

  @override
  Widget build(BuildContext context) {
    return const MaterialApp(
      title: 'Minos',
      home: Scaffold(body: Center(child: Text('Minos'))),
    );
  }
}
