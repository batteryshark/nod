#!/usr/bin/env python3
"""Source-derived isolated iOS UI fixture. No installed Nod app/runtime is used.

Requires an explicitly granted heavy slot unless --prepare-only. Creates only
its own simulator, never opens Simulator.app, and shuts down its exact UUID in
finally. Preserves device, sources, manifests, results and all caches.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import subprocess
import uuid

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--output', type=Path, required=True)
p.add_argument('--prepare-only', action='store_true')
p.add_argument('--device-type', default='com.apple.CoreSimulator.SimDeviceType.iPhone-13')
a = p.parse_args()
src = Path(__file__).resolve().parents[2]
out = a.output.resolve()
if out == src or src in out.parents:
    p.error('output must be outside the Apple source checkout')
out.mkdir(parents=True, exist_ok=True)
generated = out/'Sources'
generated.mkdir(exist_ok=True)
paths = ['Sources/NodKit/Models.swift', 'Sources/NodKit/NodAnswerValue.swift',
         'Sources/NodKit/NodAnswerState.swift', 'Sources/NodKit/NodAnswerComposition.swift',
         'Sources/NodKit/NodMarkdownParser.swift', 'Apps/Shared/RequestDetailView.swift',
         'Apps/Shared/RequestAnswerViews.swift', 'Apps/Shared/MarkdownText.swift',
         'Apps/Shared/RequestMediaViews.swift', 'Apps/Shared/AppBuildInfo.swift']
manifest = {}
for path in paths:
    text = (src/path).read_text()
    manifest[path] = hashlib.sha256(text.encode()).hexdigest()
    text = text.replace('import NodKit\n', '')
    if path.endswith('NodAnswerState.swift'):
        text = text[:text.index('extension NodStore {')]
    if path.endswith('AppBuildInfo.swift'):
        text = text[:text.index('#if os(macOS)\nfunc openNodNotificationSettings')]
    (generated/Path(path).name).write_text(text)
(generated/'IOSFixture.swift').write_text((Path(__file__).parent/'IOSFixture.swift').read_text())
test_path = out/'IOSInteractionTests.swift'
test_path.write_text((Path(__file__).parent/'IOSInteractionTests.swift').read_text())
(out/'source-manifest.json').write_text(json.dumps(manifest, indent=2))

objects = {}
def obj(isa, **kw):
    key = uuid.uuid4().hex[:24].upper()
    objects[key] = dict(isa=isa, **kw)
    return key
def configs(settings):
    c = obj('XCBuildConfiguration', name='Debug', buildSettings=settings)
    return obj('XCConfigurationList', buildConfigurations=[c], defaultConfigurationIsVisible=0, defaultConfigurationName='Debug')
def sources(files):
    refs = [obj('PBXFileReference', lastKnownFileType='sourcecode.swift', path=str(f), sourceTree='<absolute>') for f in files]
    phase = obj('PBXSourcesBuildPhase', buildActionMask=2147483647, files=[obj('PBXBuildFile', fileRef=r) for r in refs], runOnlyForDeploymentPostprocessing=0)
    return refs, phase
app_refs, app_sources = sources(sorted(generated.glob('*.swift')))
test_refs, test_sources = sources([test_path])
app_product = obj('PBXFileReference', explicitFileType='wrapper.application', path='Fixture.app', sourceTree='BUILT_PRODUCTS_DIR')
test_product = obj('PBXFileReference', explicitFileType='wrapper.cfbundle', path='FixtureUITests.xctest', sourceTree='BUILT_PRODUCTS_DIR')
products = obj('PBXGroup', name='Products', children=[app_product,test_product], sourceTree='<group>')
group = obj('PBXGroup', children=app_refs+test_refs+[products], sourceTree='<group>')
common = dict(SDKROOT='iphonesimulator', SUPPORTED_PLATFORMS='iphonesimulator', IPHONEOS_DEPLOYMENT_TARGET='17.0', SWIFT_VERSION='6.0', CODE_SIGNING_ALLOWED='NO', TARGETED_DEVICE_FAMILY='1,2', GENERATE_INFOPLIST_FILE='YES')
app_settings = dict(common, PRODUCT_NAME='Fixture', PRODUCT_BUNDLE_IDENTIFIER='com.nod.isolated-native-fixture', INFOPLIST_KEY_UILaunchScreen_Generation='YES', INFOPLIST_KEY_UISupportedInterfaceOrientations='UIInterfaceOrientationPortrait UIInterfaceOrientationLandscapeLeft UIInterfaceOrientationLandscapeRight')
fixture_info = out/'Fixture-Info.plist'
fixture_info.write_bytes(plistlib.dumps(dict(CFBundleExecutable='$(EXECUTABLE_NAME)', CFBundleIdentifier='$(PRODUCT_BUNDLE_IDENTIFIER)', CFBundleName='Fixture', CFBundlePackageType='APPL', CFBundleVersion='1', CFBundleShortVersionString='1.0', LSRequiresIPhoneOS=True, UILaunchScreen={}, UIApplicationSceneManifest=dict(UIApplicationSupportsMultipleScenes=False), UISupportedInterfaceOrientations=['UIInterfaceOrientationPortrait','UIInterfaceOrientationLandscapeLeft','UIInterfaceOrientationLandscapeRight'])))
app_settings.update(GENERATE_INFOPLIST_FILE='NO', INFOPLIST_FILE=str(fixture_info))
app = obj('PBXNativeTarget', name='Fixture', productName='Fixture', productType='com.apple.product-type.application', productReference=app_product, buildConfigurationList=configs(app_settings), buildPhases=[app_sources], buildRules=[], dependencies=[])
test_settings = dict(common, PRODUCT_NAME='FixtureUITests', PRODUCT_BUNDLE_IDENTIFIER='com.nod.isolated-native-fixture.tests', TEST_TARGET_NAME='Fixture')
test = obj('PBXNativeTarget', name='FixtureUITests', productName='FixtureUITests', productType='com.apple.product-type.bundle.ui-testing', productReference=test_product, buildConfigurationList=configs(test_settings), buildPhases=[test_sources], buildRules=[], dependencies=[])
project = obj('PBXProject', attributes=dict(LastUpgradeCheck='2650',TargetAttributes={test:dict(TestTargetID=app)}), buildConfigurationList=configs(common), compatibilityVersion='Xcode 14.0', developmentRegion='en', knownRegions=['en','Base'], mainGroup=group, productRefGroup=products, projectDirPath='', projectRoot='', targets=[app,test])
proxy = obj('PBXContainerItemProxy', containerPortal=project, proxyType=1, remoteGlobalIDString=app, remoteInfo='Fixture')
objects[test]['dependencies'] = [obj('PBXTargetDependency', target=app, targetProxy=proxy)]
project_dir = out/'Fixture.xcodeproj'
project_dir.mkdir(exist_ok=True)
(project_dir/'project.pbxproj').write_bytes(plistlib.dumps(dict(archiveVersion='1',classes={},objectVersion='56',objects=objects,rootObject=project)))
schemes = project_dir/'xcshareddata/xcschemes'
schemes.mkdir(parents=True,exist_ok=True)
def ref(key,name):
    return f'<BuildableReference BuildableIdentifier="primary" BlueprintIdentifier="{key}" BuildableName="{name}" BlueprintName="{name.split(".")[0]}" ReferencedContainer="container:Fixture.xcodeproj"/>'
(schemes/'Fixture.xcscheme').write_text(f'''<?xml version="1.0" encoding="UTF-8"?><Scheme LastUpgradeVersion="2650" version="1.3"><BuildAction parallelizeBuildables="NO" buildImplicitDependencies="YES"><BuildActionEntries><BuildActionEntry buildForTesting="YES" buildForRunning="YES" buildForProfiling="NO" buildForArchiving="NO" buildForAnalyzing="NO">{ref(app,'Fixture.app')}</BuildActionEntry></BuildActionEntries></BuildAction><TestAction buildConfiguration="Debug" selectedDebuggerIdentifier="" selectedLauncherIdentifier="Xcode.IDEFoundation.Launcher.PosixSpawn" shouldUseLaunchSchemeArgsEnv="YES"><Testables><TestableReference skipped="NO">{ref(test,'FixtureUITests.xctest')}</TestableReference></Testables></TestAction></Scheme>''')
if a.prepare_only:
    print('Prepared source-derived inert fixture; no compiler/simulator started')
    raise SystemExit(0)

env = dict(os.environ, DEVELOPER_DIR='/Applications/Xcode.app/Contents/Developer')
device = subprocess.check_output(['xcrun','simctl','create','Nod owned inert fixture',a.device_type,'com.apple.CoreSimulator.SimRuntime.iOS-27-0'],env=env,text=True).strip()
ownership = dict(device=device,deviceType=a.device_type,app='com.nod.isolated-native-fixture',headless=True,shutdown=False)
(out/'ownership.json').write_text(json.dumps(ownership,indent=2))
try:
    subprocess.run(['xcrun','simctl','boot',device],env=env,check=True,timeout=60)
    subprocess.run(['xcrun','simctl','bootstatus',device,'-b'],env=env,check=True,timeout=180)
    with (out/'xcode-test.log').open('w') as log:
        result = subprocess.run(['xcodebuild','-project',str(project_dir),'-scheme','Fixture','-destination','id='+device,'-derivedDataPath',str(out/'DerivedData'),'-resultBundlePath',str(out/'Results.xcresult'),'-parallel-testing-enabled','NO','-jobs','2','OTHER_SWIFT_FLAGS=$(inherited) -j2','CODE_SIGNING_ALLOWED=NO','test'],env=env,stdout=log,stderr=subprocess.STDOUT,timeout=600)
    raise SystemExit(result.returncode)
finally:
    shutdown = subprocess.run(['xcrun','simctl','shutdown',device],env=env,timeout=60)
    ownership['shutdown'] = shutdown.returncode == 0
    (out/'ownership.json').write_text(json.dumps(ownership,indent=2))
