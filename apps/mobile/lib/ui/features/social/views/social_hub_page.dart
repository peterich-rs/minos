import 'package:flutter/material.dart';
import 'package:minos/ui/features/messages/views/messages_page.dart';

/// Legacy route entry for `/social`. Redirects to the Messages golden path.
class SocialHubPage extends StatelessWidget {
  const SocialHubPage({super.key, this.showAppBar = true});

  /// Kept for call-site compatibility; [MessagesPage] owns its own chrome.
  final bool showAppBar;

  @override
  Widget build(BuildContext context) => const MessagesPage();
}
