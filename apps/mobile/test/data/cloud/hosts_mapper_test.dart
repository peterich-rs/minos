import 'package:flutter_test/flutter_test.dart';
import 'package:minos/data/cloud/hosts_mapper.dart';

void main() {
  group('mapHostsListResponse', () {
    test('maps ResponseEnvelope hosts from GET /v1/hosts', () {
      final hosts = mapHostsListResponse({
        'data': {
          'hosts': [
            {
              'host_installation_id': '11111111-1111-1111-1111-111111111111',
              'host_display_name': 'MacBook Pro',
              'linked_at_ms': 1700000000000,
              'online': true,
            },
            {
              'host_installation_id': '22222222-2222-2222-2222-222222222222',
              'host_display_name': 'Studio',
              'linked_at_ms': 1700000001000,
              'online': false,
            },
          ],
        },
        'request_id': 'req-1',
      });

      expect(hosts, hasLength(2));
      expect(
        hosts[0].hostInstallationId,
        '11111111-1111-1111-1111-111111111111',
      );
      expect(hosts[0].hostDisplayName, 'MacBook Pro');
      expect(hosts[0].linkedAtMs, 1700000000000);
      expect(hosts[0].online, isTrue);
      expect(hosts[1].online, isFalse);
    });

    test('maps bare hosts array without envelope', () {
      final hosts = mapHostsListResponse({
        'hosts': [
          {
            'host_installation_id': 'aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa',
            'host_display_name': 'Bare',
            'linked_at_ms': 1,
            'online': true,
          },
        ],
      });
      expect(hosts, hasLength(1));
      expect(hosts.single.hostDisplayName, 'Bare');
    });

    test('accepts paired_at_ms alias for linked_at_ms', () {
      final host = mapHostSummaryJson({
        'host_installation_id': 'bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb',
        'host_display_name': 'Legacy',
        'paired_at_ms': 42,
        'online': false,
      });
      expect(host.linkedAtMs, 42);
    });

    test('throws when host_installation_id missing', () {
      expect(
        () => mapHostSummaryJson({
          'host_display_name': 'x',
          'linked_at_ms': 1,
          'online': true,
        }),
        throwsA(isA<FormatException>()),
      );
    });

    test('empty hosts list is valid', () {
      expect(
        mapHostsListResponse({
          'data': {'hosts': <Object>[]},
        }),
        isEmpty,
      );
    });
  });
}
