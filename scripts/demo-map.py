#!/usr/bin/env python3
"""Write a fictional two-site /mapnetwork map (demo.dc.html) for UI work:
    python3 scripts/demo-map.py demo.dc.html
    IS_ALIVE_SHOTS=shots is-alive demo.dc.html   # screenshot every tab
"""
import json, sys
nodes, edges = [], []
def n(id, zone, kind, label, sub, site='hq', **kw):
    d = dict(id=id, site=site, zone=zone, kind=kind, label=label, sub=sub); d.update(kw); nodes.append(d)
def e(a, b, cls='wired', label=None):
    d = dict(id=f'e{len(edges)}', **{'from': a}, to=b, cls=cls)
    if label: d['label'] = label
    edges.append(d)
nodes.append(dict(id='internet', zone='gateway', kind='WAN', label='Internet', sub='203.0.113.10'))
n('fw', 'gateway', 'FIREWALL', 'Edge firewall', '10.10.0.1', details=['Model TZ-demo', 'Firmware 7.1'], chips=['HA pair'])
e('internet', 'fw', 'backbone')
n('core', 'distribution', 'CORE SWITCH', 'Core switch', '10.10.0.2', chips=['48 ports'])
e('fw', 'core', 'wired', 'X2')
for i, room in enumerate(['Floor 1', 'Floor 2']):
    sid = f'sw{i}'
    n(sid, 'distribution', 'ACCESS SWITCH', f'{room} switch', f'10.10.0.{10+i}', chips=['24 ports'])
    e('core', sid, 'wired', f'port {i+1}')
    for j in range(2):
        ap = f'ap{i}{j}'
        n(ap, 'trusted', 'AP', f'{room} AP {j+1}', f'10.10.0.{20+i*2+j}')
        e(sid, ap, 'wired', f'port {j+5}')
        for k in range(2):
            c = f'c{i}{j}{k}'
            n(c, 'guest' if k else 'trusted', 'CLIENT', f'Laptop {i}{j}{k}', f'10.10.{20+i}.{100+j*10+k}', client=True)
            e(ap, c, 'wifi')
n('srv1', 'server', 'SERVER', 'File server', '10.10.5.10', chips=['inferred'], flags=['Telnet open'])
n('srv2', 'server', 'SERVER', 'Hypervisor', '10.10.5.11')
e('core', 'srv1', 'wired', 'port 20'); e('core', 'srv2', 'wired', 'port 21')
n('prn', 'iot', 'PRINTER', 'Office printer', '10.10.30.5', flags=['Plain HTTP management'])
n('cam1', 'iot', 'CAMERA', 'Lobby camera', '10.10.30.20')
n('cam2', 'iot', 'CAMERA', 'Car park camera', '10.10.30.21', chips=['no answer'])
for x in ['prn', 'cam1', 'cam2']: e('sw0', x, 'wired')
n('unk', 'unknown', 'UNKNOWN DEVICE', 'Unidentified upstream device', '58:11:22:aa:bb:cc', chips=['not managed'])
e('sw1', 'unk', 'wired', 'port 24')
# second site
n('fw2', 'gateway', 'FIREWALL', 'Branch firewall', '10.20.0.1', site='br')
e('internet', 'fw2', 'backbone')
n('sw2', 'distribution', 'ACCESS SWITCH', 'Branch switch', '10.20.0.2', site='br')
e('fw2', 'sw2', 'wired', 'X1')
n('ap2', 'trusted', 'AP', 'Branch AP', '10.20.0.3', site='br'); e('sw2', 'ap2', 'wired', 'port 3')
n('pos', 'iot', 'POS', 'Till', '10.20.30.4', site='br'); e('sw2', 'pos', 'wired', 'port 7')
zones = {
 'gateway': dict(name='Gateway / firewall', vlan='edge'), 'distribution': dict(name='Switches', vlan='wired'),
 'server': dict(name='Servers', vlan='VLAN 5'), 'trusted': dict(name='Staff', vlan='VLAN 20'),
 'iot': dict(name='IoT', vlan='VLAN 30'), 'guest': dict(name='Guest', vlan='VLAN 40'), 'unknown': dict(name='Unknown', vlan='?'),
}
actions = [
 dict(id='a1', sev='high', text='Telnet is open on File server (10.10.5.10): turn it off; use SSH.', nodes=['srv1']),
 dict(id='a2', sev='med', text='Floor 2 switch uplinks to an unmanaged device on port 24: identify it on site.', nodes=['unk']),
 dict(id='a3', sev='low', text='Office printer serves its management page over plain HTTP: enable HTTPS.', nodes=['prn']),
 dict(id='a4', sev='low', text='Car park camera is in the swept range but did not answer.', nodes=['cam2']),
]
J = lambda v: json.dumps(v, ensure_ascii=False)
page = f'''<html><script>
class X {{ constructor() {{
/* ===================== SITE DATA ===================== */
this.META = {{ title: 'Demo Co — Network', subtitle: "2 sites · fictional demo data" }};
this.SITES = {J({'hq': {'name': 'Head office'}, 'br': {'name': 'Branch'}})};
this.ZONES = {J(zones)};
this.NODES = {J(nodes)};
this.EDGES = {J(edges)};
this.ACTIONS = {J(actions)};
this.BANDS = [];
/* =================== END SITE DATA =================== */
}} }}
</script></html>'''
open(sys.argv[1], 'w').write(page)
print(len(nodes), 'nodes')
