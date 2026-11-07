#!/usr/bin/env python3
"""Check all bundled engines against the native map's theme and flow contract."""
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[3]
native = (ROOT / 'src/map/mod.rs').read_text()
checks = 0
for path in (ROOT / 'skills/mapnetwork/assets').glob('*.dc.html'):
    text = path.read_text()
    method = text.split('  themeDefs(){', 1)[1].split('\n  pal(){', 1)[0]
    expression = re.search(r"style = (`animation:flowDash[^\n]+);", text).group(1)
    js = '''
const object = {ZONES: {
 gateway:{dark:'#5b9dff'}, distribution:{dark:'#a98bff'},
 access:{dark:'#5fcf8a'}, devices:{dark:'#f0944a'}, servers:{dark:'#e0b34d'}
}, themeDefs(){''' + method + '''};
const themes = object.themeDefs();
const flow = downstream => ''' + expression + ''';
console.log(JSON.stringify({themes, forward:flow(true), backward:flow(false)}));
'''
    data = json.loads(subprocess.check_output(['node', '-e', js], text=True))
    for theme in ['Slate', 'Phosphor', 'Blueprint', 'Contrast', 'Rgb', 'Paper']:
        body = re.search(r'Theme::' + theme + r' => \[(.*?)\]', native, re.S).group(1)
        colors = ['#' + value.lower() for value in re.findall(r'0x([0-9a-fA-F]{6})', body)]
        actual = data['themes'][theme.lower()]['zones']
        for role, index in [('gateway', 1), ('distribution', 2), ('access', 3), ('devices', 6), ('servers', 5)]:
            assert actual[role] == colors[index], (path.name, theme, role)
            checks += 1
    assert 'animation-direction:reverse' in data['forward']
    assert 'animation-direction:reverse' not in data['backward']
    assert 'prefers-reduced-motion: reduce' in text and 'transition:none !important' in text
    assert "removeEventListener('visibilitychange', this._motionVisibility)" in text
    print('PASS', path.name)
print(f'{checks} native/HTML role-color comparisons passed; uplink direction and motion lifecycle checked in all engines')
