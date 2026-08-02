import 'dart:convert';

import 'package:http/http.dart' as http;
import 'package:minos/data/cloud/auth_exchange_mapper.dart';
import 'package:minos/data/cloud/cloud_config.dart';
import 'package:minos/data/cloud/hosts_mapper.dart';
import 'package:minos/domain/linked_host.dart';
import 'package:minos/domain/minos_session.dart';

/// Pure-Dart HTTP client for Minos cloud control-plane endpoints used by the
/// mobile data plane (exchange + hosts list). Session stream/send still go
/// through the Rust FRB client over `/ws/client`.
///
/// Intentionally free of Flutter/FRB so repositories and mappers stay
/// unit-testable under `flutter test --exclude-tags ffi`.
class MinosCloudClient {
  MinosCloudClient({
    required CloudConfig config,
    http.Client? httpClient,
    this.deviceRole = 'mobile-client',
    this.deviceName = 'iPhone',
  }) : _config = config,
       _http = httpClient ?? http.Client();

  final CloudConfig _config;
  final http.Client _http;
  final String deviceRole;
  final String deviceName;

  String get httpBase => _config.httpBase;

  /// `POST /v1/auth/supabase` — exchange Supabase JWT → Minos session.
  Future<MinosSession> exchangeSupabase({
    required String deviceId,
    required String supabaseAccessToken,
    String? deviceName,
  }) async {
    final body = supabaseExchangeRequestBody(
      accessToken: supabaseAccessToken,
      deviceName: deviceName ?? this.deviceName,
    );
    final json = await _requestJson(
      method: 'POST',
      path: '/v1/auth/supabase',
      deviceId: deviceId,
      body: body,
    );
    return mapAuthResponse(json);
  }

  /// `GET /v1/hosts` — linked hosts for the authenticated account.
  Future<List<LinkedHost>> listHosts({
    required String deviceId,
    required String accessToken,
  }) async {
    final json = await _requestJson(
      method: 'GET',
      path: '/v1/hosts',
      deviceId: deviceId,
      accessToken: accessToken,
    );
    return mapHostsListResponse(json);
  }

  Future<Object?> _requestJson({
    required String method,
    required String path,
    required String deviceId,
    String? accessToken,
    Map<String, dynamic>? body,
  }) async {
    final uri = Uri.parse('$httpBase$path');
    final headers = <String, String>{
      'content-type': 'application/json',
      'accept': 'application/json',
      'x-device-id': deviceId,
      'x-device-role': deviceRole,
      'x-device-name': deviceName,
    };
    if (accessToken != null && accessToken.isNotEmpty) {
      headers['authorization'] = 'Bearer $accessToken';
    }

    final http.Response response;
    switch (method) {
      case 'GET':
        response = await _http.get(uri, headers: headers);
      case 'POST':
        response = await _http.post(
          uri,
          headers: headers,
          body: body == null ? null : jsonEncode(body),
        );
      default:
        throw UnsupportedError('HTTP method $method');
    }

    if (response.statusCode < 200 || response.statusCode >= 300) {
      throw MinosCloudHttpException(
        statusCode: response.statusCode,
        body: response.body,
        path: path,
      );
    }
    if (response.statusCode == 204 || response.body.isEmpty) {
      return null;
    }
    return jsonDecode(response.body);
  }
}

class MinosCloudHttpException implements Exception {
  MinosCloudHttpException({
    required this.statusCode,
    required this.body,
    required this.path,
  });

  final int statusCode;
  final String body;
  final String path;

  String? get code {
    try {
      final decoded = jsonDecode(body);
      if (decoded is Map) {
        final err = decoded['error'];
        if (err is Map && err['code'] is String) {
          return err['code'] as String;
        }
      }
    } catch (_) {}
    return null;
  }

  @override
  String toString() =>
      'MinosCloudHttpException($statusCode $path${code == null ? '' : ' code=$code'})';
}
