import 'package:flutter/material.dart';

import 'src/rust/frb_generated.dart';

void main() async {
  // Required so `RustLib.init()` can use the isolate infrastructure before
  // `runApp` kicks off the first frame.
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
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
