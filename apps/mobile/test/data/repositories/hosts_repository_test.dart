import 'package:flutter_test/flutter_test.dart';
import 'package:minos/data/repositories/hosts_repository.dart';
import 'package:minos/domain/linked_host.dart';

void main() {
  group('linkedHostToDto', () {
    test('maps domain host to FRB HostSummaryDto shape', () {
      const host = LinkedHost(
        hostInstallationId: '11111111-1111-1111-1111-111111111111',
        hostDisplayName: 'Mac',
        linkedAtMs: 99,
        online: true,
      );
      final dto = linkedHostToDto(host);
      expect(dto.hostDeviceId, host.hostInstallationId);
      expect(dto.hostDisplayName, 'Mac');
      expect(dto.online, isTrue);
      expect(dto.pairedViaDeviceId, '00000000-0000-0000-0000-000000000000');
    });
  });
}
