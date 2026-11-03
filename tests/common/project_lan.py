import errno, struct, subprocess, time

blocked = [('-4', '10.200.0.1'), ('-4', '172.31.255.255'),
           ('-4', '192.168.255.254'), ('-4', '100.64.0.1'),
           ('-4', '169.254.9.9'), ('-6', 'fd00::1'), ('-6', 'fe80::5')]
routed = [('-4', '172.32.0.1'), ('-4', '100.128.0.1'), ('-4', '1.1.1.1')]
lan = dict(blocked_routes=[], routed=[], connects=[])
for family, address in blocked + routed:
    status = subprocess.run(['ip', family, 'route', 'get', address],
                            capture_output=True).returncode
    lan['blocked_routes' if (family, address) in blocked else 'routed'].append(status)
for family, address in blocked:
    af = socket.AF_INET if family == '-4' else socket.AF_INET6
    scope = next((i for i, name in socket.if_nameindex() if name != 'lo'), 0)
    target = (address, 80) if af == socket.AF_INET else (address, 80, 0, scope if address.startswith('fe80') else 0)
    with socket.socket(af, socket.SOCK_STREAM) as stream:
        stream.settimeout(2)
        start = time.monotonic()
        try:
            stream.connect(target)
            code = 0
        except OSError as error:
            code = error.errno
        lan['connects'].append(code == errno.ENETUNREACH and time.monotonic() - start < 1)
forwarder = next(line.split()[1] for line in pathlib.Path('/etc/resolv.conf').read_text().splitlines()
                 if line.startswith('nameserver '))
query = struct.pack('!6H', 1234, 256, 1, 0, 0, 0) + b'\x07example\x03com\x00\x00\x01\x00\x01'
with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as dns:
    dns.settimeout(3)
    dns.connect((forwarder, 53))
    dns.send(query)
    try:
        lan['dns'] = bool(dns.recv(4096))
    except OSError:
        lan['dns'] = False
lan['delete'] = subprocess.run(['ip', '-4', 'rule', 'del', 'to', '10.0.0.0/8', 'pref', '20'],
                               capture_output=True).returncode
result['lan'] = lan
