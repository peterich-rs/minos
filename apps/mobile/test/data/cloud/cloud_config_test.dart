import 'package:flutter_test/flutter_test.dart';
import 'package:minos/data/cloud/cloud_config.dart';

void main() {
  group('CloudConfig.httpBase', () {
    test('converts wss gateway URL to https origin', () {
      const cfg = CloudConfig(
        backendUrl: 'wss://hub.example.com/devices',
        supabaseUrl: '',
        supabaseAnonKey: '',
      );
      expect(cfg.httpBase, 'https://hub.example.com');
    });

    test('converts ws dev URL to http origin', () {
      const cfg = CloudConfig(
        backendUrl: 'ws://127.0.0.1:8787/devices',
        supabaseUrl: '',
        supabaseAnonKey: '',
      );
      expect(cfg.httpBase, 'http://127.0.0.1:8787');
    });

    test('accepts bare http base with trailing slash', () {
      const cfg = CloudConfig(
        backendUrl: 'https://hub.example.com/',
        supabaseUrl: '',
        supabaseAnonKey: '',
      );
      expect(cfg.httpBase, 'https://hub.example.com');
    });

    test('isSupabaseConfigured requires both slots', () {
      const partial = CloudConfig(
        backendUrl: 'http://x',
        supabaseUrl: 'https://x.supabase.co',
        supabaseAnonKey: '',
      );
      expect(partial.isSupabaseConfigured, isFalse);

      const full = CloudConfig(
        backendUrl: 'http://x',
        supabaseUrl: 'https://x.supabase.co',
        supabaseAnonKey: 'anon',
      );
      expect(full.isSupabaseConfigured, isTrue);
    });
  });
}
