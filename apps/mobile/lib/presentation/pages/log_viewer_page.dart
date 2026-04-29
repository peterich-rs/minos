import 'package:flutter/material.dart';

import 'package:minos/presentation/widgets/log_panel.dart';
import 'package:minos/presentation/widgets/request_trace_panel.dart';

class LogViewerPage extends StatelessWidget {
  const LogViewerPage({super.key});

  @override
  Widget build(BuildContext context) {
    return DefaultTabController(
      length: 2,
      child: Scaffold(
        appBar: AppBar(
          title: const Text('Devtool'),
          bottom: const TabBar(
            tabs: <Widget>[
              Tab(text: '日志'),
              Tab(text: '请求'),
            ],
          ),
        ),
        body: SafeArea(
          child: LayoutBuilder(
            builder: (_, constraints) => TabBarView(
              children: <Widget>[
                LogPanel(height: constraints.maxHeight, showControls: true),
                RequestTracePanel(
                  height: constraints.maxHeight,
                  showControls: true,
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
