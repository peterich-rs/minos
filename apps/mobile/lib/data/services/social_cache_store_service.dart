import 'package:flutter_riverpod/flutter_riverpod.dart';

import 'package:minos/infrastructure/social_cache_store.dart';

final socialCacheStoreProvider = Provider<SocialCacheStore>((ref) {
  return SocialCacheStore();
});
