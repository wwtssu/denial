import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart' show debugPrint;
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../config/startup_environment.dart';
import 'debug_actions.dart';

/// Loopback-only debug control HTTP server for agent-driven development.
///
/// This is a development facility, decoupled from the shell's normal
/// operation: it is never started unless the session environment explicitly
/// enables it (`DENIAL_DEBUG_HTTP=1`, port override with
/// `DENIAL_DEBUG_HTTP_PORT`). It binds only to 127.0.0.1 and carries no
/// authentication — same trust class as the Dart VM service of a debug
/// bundle, intended for the local user's own tooling.
///
/// The server is deliberately generic: it operates UI targets by
/// `key` + `action` (the GlobalKey model) and exposes no per-feature routes.
/// Adding a debuggable capability means registering a
/// [DebugActionTarget], never touching this file.
class DebugControlServer {
  DebugControlServer._(this._ref, this._server);

  final WidgetRef _ref;
  final HttpServer _server;

  static const int _defaultPort = 17894;

  /// Starts the server; returns `null` when the debug facility is disabled.
  static Future<DebugControlServer?> start(WidgetRef ref) async {
    final environment = ref.read(startupEnvironmentProvider);
    if (!environment.flag('DENIAL_DEBUG_HTTP')) {
      return null;
    }
    final port =
        int.tryParse(
          environment.values['DENIAL_DEBUG_HTTP_PORT']?.trim() ?? '',
        ) ??
        _defaultPort;
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, port);
    final control = DebugControlServer._(ref, server);
    server.listen(control._handle);
    debugPrint('denia debug control: http://127.0.0.1:$port/api/health');
    return control;
  }

  Future<void> close() async {
    await _server.close(force: true);
  }

  Future<void> _handle(HttpRequest request) async {
    try {
      final path = request.uri.path;
      final method = request.method;
      if (method == 'GET' && path == '/api/health') {
        await _respondJson(request, <String, Object?>{'ok': true});
      } else if (method == 'GET' && path == '/api/keys') {
        await _listTargets(request);
      } else if (method == 'POST' && path == '/api/ui') {
        await _invokeTarget(request);
      } else if (method == 'GET' && path == '/api/state') {
        await _invokeTarget(request, key: 'state', action: 'get');
      } else {
        request.response.statusCode = HttpStatus.notFound;
        await _respondJson(
          request,
          <String, Object?>{'error': 'not found', 'path': path},
        );
      }
    } on DebugActionException catch (error) {
      request.response.statusCode = HttpStatus.badRequest;
      await _respondJson(request, <String, Object?>{'error': '$error'});
    } on Object catch (error) {
      request.response.statusCode = HttpStatus.internalServerError;
      await _respondJson(request, <String, Object?>{'error': '$error'});
    } finally {
      await request.response.close();
    }
  }

  Future<void> _listTargets(HttpRequest request) async {
    final registry = _ref.read(debugActionsProvider);
    final targets = <Map<String, Object?>>[
      for (final key in registry.keys)
        <String, Object?>{
          'key': key,
          'actions': registry.actionsFor(key)?.toList() ?? const <String>[],
        },
    ];
    await _respondJson(
      request,
      <String, Object?>{'count': targets.length, 'targets': targets},
    );
  }

  Future<void> _invokeTarget(
    HttpRequest request, {
    String? key,
    String? action,
  }) async {
    final registry = _ref.read(debugActionsProvider);
    final body = await _readBody(request);
    final targetKey = key ?? (body['key'] as String?)?.trim();
    final target = targetKey == null ? null : registry.targetFor(targetKey);
    if (target == null) {
      request.response.statusCode = HttpStatus.notFound;
      await _respondJson(
        request,
        <String, Object?>{'error': 'unknown key', 'key': targetKey},
      );
      return;
    }
    final targetAction = action ?? (body['action'] as String?)?.trim();
    final args = body['args'];
    final result = await target.invoke(
      DebugActionContext(_ref),
      targetAction ?? '',
      args is Map<String, Object?> ? args : const <String, Object?>{},
    );
    await _respondJson(request, result);
  }

  Future<Map<String, Object?>> _readBody(HttpRequest request) async {
    final text = await utf8.decoder.bind(request).join();
    if (text.trim().isEmpty) {
      return const <String, Object?>{};
    }
    final decoded = jsonDecode(text);
    return decoded is Map<String, Object?> ? decoded : const <String, Object?>{};
  }

  Future<void> _respondJson(HttpRequest request, Object body) async {
    request.response.headers.contentType = ContentType.json;
    request.response.write(jsonEncode(body));
  }
}
