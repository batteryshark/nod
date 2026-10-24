#!/usr/bin/env python3
"""Offscreen native regression. Requires Xcode; never loads the real NodStore.

Usage: python3 Tests/AppLayout/run.py --output /absolute/scratch
Optional --root-source supplies a baseline NodRootViews.swift for a negative control.
All generated source, caches and JSON stay outside the source checkout.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--output', type=Path, required=True)
p.add_argument('--root-source', type=Path)
p.add_argument('--compile-only', action='store_true', help='Compile source-derived probe without any window/process execution')
a = p.parse_args()
src = Path(__file__).resolve().parents[2]
out = a.output.resolve()
if out == src or src in out.parents:
    p.error('--output must be outside the Apple source checkout')
out.mkdir(parents=True, exist_ok=True)
files = ['Sources/NodKit/Models.swift', 'Sources/NodKit/NodMarkdownParser.swift',
         'Sources/NodKit/NodAnswerValue.swift',
         'Sources/NodKit/NodAnswerComposition.swift',
         'Sources/NodKit/NodAnswerState.swift', 'Sources/NodKit/NodRequestInbox.swift',
          'Apps/Shared/RequestDetailView.swift', 'Apps/Shared/RequestAnswerViews.swift', 'Apps/Shared/MarkdownText.swift',
         'Apps/Shared/RequestMediaViews.swift', 'Apps/Shared/AppBuildInfo.swift']
manifest = {}
for f in files:
    text = (src/f).read_text()
    manifest[f] = hashlib.sha256(text.encode()).hexdigest()
    text = text.replace('import NodKit\n', '')
    if f.endswith('NodAnswerState.swift'):
        text = text[:text.index('extension NodStore {')]
    if f.endswith('AppBuildInfo.swift'):
        text = text[:text.index('#if os(macOS)\nfunc openNodNotificationSettings')]
    if f.endswith('RequestAnswerViews.swift'):
        assert text.count('.disabled(isSubmitting)\n') == 2
        text = text.replace('.disabled(isSubmitting)\n', '.disabled(isSubmitting)\n        .background(Marker(id: option.id))\n', 1)
        assert text.count('.accessibilityIdentifier("answer-access")') == 1
        text = text.replace('.accessibilityIdentifier("answer-access")', '.accessibilityIdentifier("answer-access")\n        .background(Marker(id: "answer-access"))')
    (out/Path(f).name).write_text(text)
root = (a.root_source or src/'Apps/Shared/NodRootViews.swift').read_text()
selection = (src/'Apps/Shared/RequestListViews.swift').read_text()
manifest['Apps/Shared/RequestListViews.swift'] = hashlib.sha256(selection.encode()).hexdigest()
selection = selection[selection.index('struct RequestDetailContainer: View'):selection.index('struct ChannelLabel: View')]
(out/'Selection.swift').write_text('import SwiftUI\n' + selection)
manifest['Apps/Shared/NodRootViews.swift'] = hashlib.sha256(root.encode()).hexdigest()
# Extract actual root layout BEFORE lifecycle modifiers. No network task is run.
body = root.split('var body: some View {', 1)[1].split('    .task {', 1)[0]
if 'private var inboxContent:' not in root:
    # Baseline root has inline registration content and its inset AFTER task/URL.
    start = body.index('      if store.isRegistered {')
    body = body[:start] + '      inboxContent\n    }\n'
    inset = root.split('    .safeAreaInset(edge: .bottom) {',1)[1].split('    .alert(',1)[0]
    body += '    .safeAreaInset(edge: .bottom) {' + inset
body = body.replace('ConnectionStatusView()', 'ConnectionStatusView().background(Marker(id: "status-bar"))')
(out/'Root.swift').write_text('import SwiftUI\nstruct ProbeRoot: View {\n @EnvironmentObject var store: NodStore\n let inboxContent: AnyView\n var body: some View {' + body + '\n }\n}\n' + root[root.index('struct ConnectionStatusView: View'):])
(out/'Probe.swift').write_text((Path(__file__).parent/'Probe.swift').read_text())
(out/'source-manifest.json').write_text(json.dumps(manifest,indent=2))
env = dict(os.environ, DEVELOPER_DIR='/Applications/Xcode.app/Contents/Developer')
developer = Path(env['DEVELOPER_DIR'])
sdk = developer/'Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk'
compiler = developer/'Toolchains/XcodeDefault.xctoolchain/usr/bin/swiftc'
subprocess.run([str(compiler),'-sdk',str(sdk),'-j','2','-module-cache-path',str(out/'module-cache'), *map(str,out.glob('*.swift')),'-o',str(out/'probe')],env=env,check=True)
if a.compile_only:
    print('Source-derived probe compiled; no window or probe process started')
    raise SystemExit(0)
cases = [('2-short-min',['2','860','560','0','--split']),
 ('3-long-compact',['3','320','300','8','--long']),
 ('12-long-min',['12','860','560','8','--split','--long']),
 ('20-long-min',['20','860','560','8','--split','--long']),
 ('20-long-resize',['20','860','560','8','--split','--long','--resize']),
 ('20-long-large',['20','1200','800','12','--split','--long']),
 ('20-long-narrow',['20','320','560','8','--long']),
 ('12-short-min',['12','860','560','8','--split'])]
passed = True
summary = []
for name,args in cases:
    run = subprocess.run([str(out/'probe'),*args,'--png',str(out/(name+'.png'))],capture_output=True,text=True,check=True,timeout=15)
    d = json.loads(run.stdout)
    (out/(name+'.json')).write_text(run.stdout)
    n = int(args[0])
    if d.get('mode') == 'compact-access':
        ok = d['accessReachable'] and not d['visibleWindow']
        summary.append(dict(fixture=name,passed=ok,mode='compact-access'))
        passed &= ok
        print(name, 'PASS' if ok else 'FAIL',flush=True)
        continue
    ok = (d['everyOptionReachableWithoutStatusOcclusion'] and d['orderPreserved']
          and d['ids'] == ['option-'+str(i) for i in range(1,n+1)]
          and d['markedOptionCount'] == n and not d['visibleWindow']
          and d['barInHost']['height'] > 0
          and d['host']['width'] == float(args[1]) and d['host']['height'] == float(args[2]))
    summary.append(dict(fixture=name,passed=ok))
    passed &= ok
    print(name, 'PASS' if ok else 'FAIL',flush=True)
(out/'summary.json').write_text(json.dumps(summary,indent=2))
raise SystemExit(0 if passed else 1)
