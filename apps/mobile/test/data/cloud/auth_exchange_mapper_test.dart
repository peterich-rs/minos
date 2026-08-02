import 'package:flutter_test/flutter_test.dart';
import 'package:minos/data/cloud/auth_exchange_mapper.dart';

void main() {
  group('mapAuthResponse', () {
    test('maps supabase/login auth payload', () {
      final session = mapAuthResponse({
        'account': {'account_id': 'acct-1', 'email': 'user@example.com'},
        'access_token': 'access',
        'refresh_token': 'refresh',
        'expires_in': 900,
      }, nowMs: 1_000_000);

      expect(session.accountId, 'acct-1');
      expect(session.email, 'user@example.com');
      expect(session.accessToken, 'access');
      expect(session.refreshToken, 'refresh');
      expect(session.accessExpiresAtMs, 1_000_000 + 900 * 1000);
    });

    test('throws on incomplete tuple', () {
      expect(
        () => mapAuthResponse({
          'account': {'account_id': 'a', 'email': 'e'},
          'access_token': '',
          'refresh_token': 'r',
          'expires_in': 1,
        }),
        throwsA(isA<FormatException>()),
      );
    });
  });

  group('supabaseExchangeRequestBody', () {
    test('includes optional device_name when non-empty', () {
      expect(
        supabaseExchangeRequestBody(
          accessToken: 'tok',
          deviceName: '  iPhone  ',
        ),
        {'access_token': 'tok', 'device_name': 'iPhone'},
      );
    });

    test('omits blank device_name', () {
      expect(
        supabaseExchangeRequestBody(accessToken: 'tok', deviceName: '  '),
        {'access_token': 'tok'},
      );
    });
  });
}
