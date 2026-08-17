import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

void main() => runApp(const AgentBellApp());

class AgentBellApp extends StatelessWidget {
  const AgentBellApp({super.key});
  @override
  Widget build(BuildContext context) => MaterialApp(
    debugShowCheckedModeBanner: false,
    title: 'AgentBell',
    theme: ThemeData(
      useMaterial3: true,
      colorScheme: ColorScheme.fromSeed(
        seedColor: const Color(0xff087f72),
        brightness: Brightness.light,
      ),
      scaffoldBackgroundColor: const Color(0xfff4f8f6),
      cardTheme: const CardThemeData(
        elevation: 0,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.all(Radius.circular(8)),
        ),
        color: Colors.white,
      ),
      inputDecorationTheme: const InputDecorationTheme(
        border: OutlineInputBorder(
          borderRadius: BorderRadius.all(Radius.circular(8)),
        ),
      ),
    ),
    home: const HomeScreen(),
  );
}

class PcPeer {
  PcPeer(this.id, this.name, this.ip, this.port);
  final String id, name, ip;
  final int port;
  String get url => 'http://$ip:$port';
}

class HomeScreen extends StatefulWidget {
  const HomeScreen({super.key});
  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

class _HomeScreenState extends State<HomeScreen> {
  static const native = MethodChannel('agentbell/native');
  final peers = <String, PcPeer>{};
  final manual = TextEditingController();
  RawDatagramSocket? socket;
  Timer? beaconTimer, approvalTimer, subnetTimer;
  String deviceName = 'ANDROID';
  String deviceId = 'android';
  String status = '正在发现同一 Wi-Fi 下的电脑';
  String? pairCode, connectedUrl, detail;
  bool notificationGranted = false;
  bool nearbyGranted = false;
  bool batteryOptimizationIgnored = false;
  bool busy = false;
  bool scanningSubnet = false;

  @override
  void initState() {
    super.initState();
    _initialize();
  }

  Future<void> _initialize() async {
    final info = Map<String, dynamic>.from(
      await native.invokeMethod('deviceInfo'),
    );
    final maker = '${info['manufacturer'] ?? ''}'.trim();
    final model = '${info['model'] ?? ''}'.trim();
    deviceName = model.startsWith(maker) || maker.isEmpty
        ? model
        : '$maker $model';
    deviceId = '${info['id'] ?? 'android'}';
    if (info['emulator'] == true) {
      manual.text = 'http://10.0.2.2:43821';
    }
    notificationGranted =
        await native.invokeMethod<bool>('notificationGranted') ?? false;
    nearbyGranted = await native.invokeMethod<bool>('nearbyGranted') ?? false;
    batteryOptimizationIgnored =
        await native.invokeMethod<bool>('batteryOptimizationIgnored') ?? false;
    final saved = Map<String, dynamic>.from(
      await native.invokeMethod('savedConnection'),
    );
    if (saved['enabled'] == true &&
        '${saved['url']}'.isNotEmpty &&
        '${saved['token']}'.isNotEmpty) {
      final valid = await _validateSavedConnection(
        '${saved['url']}',
        '${saved['token']}',
      );
      if (valid) {
        connectedUrl = '${saved['url']}';
        status = '已连接，后台通知服务运行中';
      } else {
        await native.invokeMethod('stopService');
        status = '原连接已失效，请重新连接电脑';
      }
    }
    if (mounted) setState(() {});
    await native.invokeMethod('acquireMulticast');
    await _startDiscovery();
    Timer(const Duration(seconds: 3), _scanSubnets);
    subnetTimer = Timer.periodic(
      const Duration(seconds: 20),
      (_) => _scanSubnets(),
    );
  }

  Future<bool> _validateSavedConnection(String base, String token) async {
    try {
      final client = HttpClient()
        ..connectionTimeout = const Duration(seconds: 2);
      final request = await client.getUrl(
        Uri.parse('$base/api/status?token=${Uri.encodeQueryComponent(token)}'),
      );
      final response = await request.close().timeout(
        const Duration(seconds: 2),
      );
      if (response.statusCode != 200) return false;
      final data = jsonDecode(await utf8.decodeStream(response));
      return data['authorized'] == true;
    } catch (_) {
      return false;
    }
  }

  Future<void> _scanSubnets() async {
    if (scanningSubnet || connectedUrl != null || peers.isNotEmpty) return;
    scanningSubnet = true;
    try {
      final interfaces = await NetworkInterface.list(
        type: InternetAddressType.IPv4,
        includeLoopback: false,
      );
      final prefixes = <String>{};
      for (final interface in interfaces) {
        final name = interface.name.toLowerCase();
        if (name.contains('rmnet') ||
            name.contains('tun') ||
            name.contains('vpn')) {
          continue;
        }
        for (final address in interface.addresses) {
          final parts = address.address.split('.');
          if (parts.length != 4) continue;
          final first = int.tryParse(parts[0]) ?? 0;
          final second = int.tryParse(parts[1]) ?? 0;
          final private =
              first == 10 ||
              (first == 172 && second >= 16 && second <= 31) ||
              (first == 192 && second == 168);
          if (private) prefixes.add(parts.take(3).join('.'));
        }
      }
      for (final prefix in prefixes) {
        for (var start = 1; start < 255 && peers.isEmpty; start += 32) {
          await Future.wait([
            for (var host = start; host < start + 32 && host < 255; host++)
              _probePc('$prefix.$host'),
          ]);
        }
      }
    } finally {
      scanningSubnet = false;
    }
  }

  Future<void> _probePc(String ip) async {
    Socket? probe;
    try {
      probe = await Socket.connect(
        ip,
        43821,
        timeout: const Duration(milliseconds: 280),
      );
      probe.destroy();
      final client = HttpClient()
        ..connectionTimeout = const Duration(seconds: 1);
      final request = await client.getUrl(
        Uri.parse('http://$ip:43821/api/status'),
      );
      final response = await request.close().timeout(
        const Duration(seconds: 1),
      );
      if (response.statusCode != 200) return;
      final data = jsonDecode(await utf8.decodeStream(response));
      if (!mounted) return;
      setState(() {
        peers['tcp-$ip'] = PcPeer(
          'tcp-$ip',
          '${data['device_name'] ?? 'AgentBell PC'}',
          ip,
          43821,
        );
        status = '已找到同一 Wi-Fi 下的电脑';
        detail = 'TCP 兜底扫描已找到 $ip';
      });
    } catch (_) {
      probe?.destroy();
    }
  }

  Future<void> _startDiscovery() async {
    try {
      socket = await RawDatagramSocket.bind(
        InternetAddress.anyIPv4,
        43820,
        reuseAddress: true,
      );
      socket!.broadcastEnabled = true;
      socket!.multicastHops = 1;
      socket!.joinMulticast(InternetAddress('239.255.83.21'));
      socket!.listen((event) {
        if (event != RawSocketEvent.read) return;
        final dg = socket!.receive();
        if (dg == null) return;
        _readBeacon(
          utf8.decode(dg.data, allowMalformed: true),
          dg.address.address,
        );
      });
      void send() {
        final body =
            'AGENTBELL1|ver=1|role=mobile|dev=$deviceId|inst=$deviceId|seq=${DateTime.now().millisecondsSinceEpoch}|port=0|name=$deviceName|model=$deviceName';
        final bytes = utf8.encode(body);
        socket?.send(bytes, InternetAddress('239.255.83.21'), 43820);
        socket?.send(bytes, InternetAddress('255.255.255.255'), 43820);
      }

      send();
      beaconTimer = Timer.periodic(const Duration(seconds: 2), (_) => send());
    } catch (e) {
      if (mounted) setState(() => status = '自动发现暂不可用，可输入电脑地址连接');
    }
  }

  void _readBeacon(String text, String ip) {
    if (!text.startsWith('AGENTBELL1|')) return;
    final map = <String, String>{};
    for (final part in text.split('|').skip(1)) {
      final at = part.indexOf('=');
      if (at > 0) map[part.substring(0, at)] = part.substring(at + 1);
    }
    if (map['role'] != 'pc' || map['dev'] == null) return;
    final port = int.tryParse(map['port'] ?? '') ?? 43821;
    setState(
      () => peers[map['dev']!] = PcPeer(
        map['dev']!,
        map['name'] ?? 'AgentBell PC',
        ip,
        port,
      ),
    );
  }

  Future<void> _requestPermission() async {
    await native.invokeMethod('requestNotifications');
    await Future.delayed(const Duration(milliseconds: 500));
    notificationGranted =
        await native.invokeMethod<bool>('notificationGranted') ?? false;
    if (mounted) setState(() {});
  }

  Future<void> _requestNearby() async {
    await native.invokeMethod('requestNearby');
    await Future.delayed(const Duration(milliseconds: 700));
    nearbyGranted = await native.invokeMethod<bool>('nearbyGranted') ?? false;
    await native.invokeMethod('acquireMulticast');
    if (mounted) setState(() {});
    await _scanSubnets();
  }

  Future<void> _requestBackgroundMode() async {
    await native.invokeMethod('requestBackgroundMode');
    await Future.delayed(const Duration(seconds: 1));
    batteryOptimizationIgnored =
        await native.invokeMethod<bool>('batteryOptimizationIgnored') ?? false;
    if (mounted) setState(() {});
  }

  Future<void> _connect(String rawUrl) async {
    var base = rawUrl.trim();
    if (base.isEmpty) {
      setState(() {
        status = '请输入电脑局域网地址';
        detail = '电脑页面“设备”中会显示可用地址，例如 http://192.168.x.x:43821';
      });
      return;
    }
    if (!base.startsWith('http://') && !base.startsWith('https://')) {
      base = 'http://$base';
    }
    base = base.replaceAll(RegExp(r'/$'), '');
    setState(() {
      busy = true;
      status = '正在请求电脑批准';
      detail = base;
      pairCode = null;
    });
    try {
      final client = HttpClient()
        ..connectionTimeout = const Duration(seconds: 3);
      final request = await client.postUrl(Uri.parse('$base/api/pair'));
      request.headers.contentType = ContentType.json;
      request.write(jsonEncode({'name': deviceName, 'device_id': deviceId}));
      final response = await request.close();
      final data =
          jsonDecode(await utf8.decodeStream(response)) as Map<String, dynamic>;
      if (response.statusCode != 200) {
        throw HttpException('HTTP ${response.statusCode}');
      }
      final token = '${data['token']}';
      final device = '${data['device_id']}';
      final cursor = '${data['cursor'] ?? ''}';
      setState(() {
        pairCode = data['code']?.toString();
        status = data['state'] == 'trusted' ? '已连接' : '请在电脑端批准此设备';
        busy = false;
      });
      if (data['state'] == 'trusted') {
        await _activate(base, token, cursor);
      } else {
        approvalTimer?.cancel();
        approvalTimer = Timer.periodic(
          const Duration(seconds: 2),
          (_) => _checkApproval(base, token, device, cursor),
        );
      }
    } catch (e) {
      if (mounted) {
        setState(() {
          busy = false;
          status = '暂时无法连接这台电脑';
          detail = '$e';
        });
      }
    }
  }

  Future<void> _checkApproval(
    String base,
    String token,
    String device,
    String cursor,
  ) async {
    try {
      final client = HttpClient()
        ..connectionTimeout = const Duration(seconds: 2);
      final req = await client.getUrl(
        Uri.parse('$base/api/status?token=${Uri.encodeQueryComponent(token)}'),
      );
      final res = await req.close();
      if (res.statusCode == 200) {
        final data = jsonDecode(await utf8.decodeStream(res));
        if (data['authorized'] == true) {
          approvalTimer?.cancel();
          await _activate(base, token, cursor);
        }
      }
    } catch (_) {}
  }

  Future<void> _activate(String base, String token, String cursor) async {
    if (!notificationGranted) await _requestPermission();
    await native.invokeMethod('startService', {
      'url': base,
      'token': token,
      'cursor': cursor,
    });
    if (mounted) {
      setState(() {
        connectedUrl = base;
        pairCode = null;
        detail = null;
        status = '已连接，后台通知服务运行中';
        busy = false;
      });
    }
  }

  Future<void> _disconnect() async {
    await native.invokeMethod('stopService');
    setState(() {
      connectedUrl = null;
      status = '正在发现同一 Wi-Fi 下的电脑';
    });
  }

  @override
  void dispose() {
    beaconTimer?.cancel();
    approvalTimer?.cancel();
    subnetTimer?.cancel();
    socket?.close();
    manual.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text(
          'AgentBell',
          style: TextStyle(fontWeight: FontWeight.w700),
        ),
        actions: [
          Padding(
            padding: const EdgeInsets.only(right: 16),
            child: Center(
              child: Text(
                deviceName,
                style: const TextStyle(fontWeight: FontWeight.w600),
              ),
            ),
          ),
        ],
      ),
      body: LayoutBuilder(
        builder: (context, box) {
          final wide = box.maxWidth >= 700;
          final showDiscovery =
              connectedUrl == null && pairCode == null && !busy;
          final content = [
            _statusPanel(),
            if (showDiscovery) _discoveryPanel(),
          ];
          return SingleChildScrollView(
            padding: EdgeInsets.symmetric(
              horizontal: wide ? 32 : 16,
              vertical: 16,
            ),
            child: Center(
              child: ConstrainedBox(
                constraints: const BoxConstraints(maxWidth: 980),
                child: wide && showDiscovery
                    ? Row(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Expanded(child: content[0]),
                          const SizedBox(width: 16),
                          Expanded(child: content[1]),
                        ],
                      )
                    : Column(
                        children: [
                          content[0],
                          if (showDiscovery) const SizedBox(height: 12),
                          if (showDiscovery) content[1],
                        ],
                      ),
              ),
            ),
          );
        },
      ),
    );
  }

  Widget _statusPanel() => Card(
    child: Padding(
      padding: const EdgeInsets.all(18),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Icon(
                connectedUrl == null
                    ? Icons.notifications_active_outlined
                    : Icons.check_circle,
                color: connectedUrl == null
                    ? const Color(0xffe48a37)
                    : const Color(0xff087f72),
              ),
              const SizedBox(width: 10),
              Expanded(
                child: Text(
                  status,
                  style: const TextStyle(
                    fontSize: 17,
                    fontWeight: FontWeight.w700,
                  ),
                ),
              ),
            ],
          ),
          if (pairCode != null)
            Padding(
              padding: const EdgeInsets.only(top: 16),
              child: Text(
                '配对码  $pairCode',
                style: const TextStyle(
                  fontSize: 25,
                  fontWeight: FontWeight.w800,
                  letterSpacing: 0,
                ),
              ),
            ),
          const SizedBox(height: 12),
          Text(
            connectedUrl ?? '打开 APK 后会通过 UDP 与 TCP 两种方式寻找电脑。',
            style: TextStyle(color: Colors.grey.shade700),
          ),
          if (detail != null)
            Padding(
              padding: const EdgeInsets.only(top: 8),
              child: SelectableText(
                detail!,
                style: TextStyle(fontSize: 12, color: Colors.grey.shade600),
              ),
            ),
          const SizedBox(height: 16),
          if (!nearbyGranted)
            Padding(
              padding: const EdgeInsets.only(bottom: 10),
              child: FilledButton.icon(
                onPressed: _requestNearby,
                icon: const Icon(Icons.wifi_find),
                label: const Text('允许局域网发现'),
              ),
            ),
          if (!notificationGranted)
            FilledButton.icon(
              onPressed: _requestPermission,
              icon: const Icon(Icons.notifications),
              label: const Text('允许系统通知'),
            )
          else
            const Row(
              children: [
                Icon(Icons.verified, size: 18, color: Color(0xff087f72)),
                SizedBox(width: 8),
                Text('系统通知权限已开启'),
              ],
            ),
          if (connectedUrl != null)
            Padding(
              padding: const EdgeInsets.only(top: 12),
              child: Wrap(
                spacing: 10,
                runSpacing: 10,
                children: [
                  if (!batteryOptimizationIgnored)
                    FilledButton.icon(
                      onPressed: _requestBackgroundMode,
                      icon: const Icon(Icons.battery_saver),
                      label: const Text('允许后台持续运行'),
                    ),
                  OutlinedButton.icon(
                    onPressed: _disconnect,
                    icon: const Icon(Icons.link_off),
                    label: const Text('断开连接'),
                  ),
                ],
              ),
            ),
        ],
      ),
    ),
  );

  Widget _discoveryPanel() => Card(
    child: Padding(
      padding: const EdgeInsets.all(18),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          const Text(
            '发现的电脑',
            style: TextStyle(fontSize: 18, fontWeight: FontWeight.w800),
          ),
          const SizedBox(height: 10),
          if (peers.isEmpty)
            const Padding(
              padding: EdgeInsets.symmetric(vertical: 14),
              child: Row(
                children: [
                  SizedBox(
                    width: 18,
                    height: 18,
                    child: CircularProgressIndicator(strokeWidth: 2),
                  ),
                  SizedBox(width: 12),
                  Text('正在扫描同一 Wi-Fi（UDP + TCP）'),
                ],
              ),
            )
          else
            ...peers.values.map(
              (p) => ListTile(
                contentPadding: EdgeInsets.zero,
                leading: const Icon(Icons.computer),
                title: Text(p.name),
                subtitle: Text(p.url),
                trailing: const Icon(Icons.chevron_right),
                onTap: busy ? null : () => _connect(p.url),
              ),
            ),
          const Divider(height: 28),
          const Text('手动连接', style: TextStyle(fontWeight: FontWeight.w700)),
          const SizedBox(height: 8),
          TextField(
            controller: manual,
            keyboardType: TextInputType.url,
            decoration: const InputDecoration(
              prefixIcon: Icon(Icons.lan),
              hintText: 'http://电脑IP:43821',
            ),
          ),
          const SizedBox(height: 10),
          FilledButton.icon(
            onPressed: busy ? null : () => _connect(manual.text),
            icon: busy
                ? const SizedBox(
                    width: 18,
                    height: 18,
                    child: CircularProgressIndicator(strokeWidth: 2),
                  )
                : const Icon(Icons.link),
            label: const Text('连接电脑'),
          ),
        ],
      ),
    ),
  );
}
