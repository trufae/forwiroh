#!/usr/bin/env python3
"""Deterministic, dependency-free project generation. Run after adding Swift files."""
import hashlib
from pathlib import Path
root = Path(__file__).resolve().parents[1] / "apps/ios"
objects = {}
def ident(name): return hashlib.sha1(name.encode()).hexdigest()[:24].upper()
def quote(s): return '"' + s.replace('\\','\\\\').replace('"','\\"').replace('\n','\\n') + '"'
def obj(name, body):
    i = ident(name); objects[i] = body; return i
def arr(items): return '(' + ', '.join(items) + (',)' if items else ')')
def config(name, settings):
    return obj(name, 'isa = XCBuildConfiguration; name = '+quote(name.split('/')[-1])+'; buildSettings = {' + ''.join(k+' = '+quote(v)+';' for k,v in settings.items()) + '};')
def configs(name, settings):
    cs = [config(name+'/'+mode, dict(settings, DEBUG_INFORMATION_FORMAT='dwarf-with-dsym' if mode=='Release' else 'dwarf', SWIFT_OPTIMIZATION_LEVEL='-O' if mode=='Release' else '-Onone')) for mode in ('Debug','Release')]
    return obj(name+'/configs','isa = XCConfigurationList; buildConfigurations = '+arr(cs)+'; defaultConfigurationIsVisible = 0; defaultConfigurationName = Release;')
products=[]; refs=[]; targets={}
for name,ext in [('PacketTunnel','appex'),('Forwiroh','app')]:
    sources=[]
    for path in sorted((root/name).glob('*.swift')) + sorted((root/'Shared').glob('*.swift')):
        rel=str(path.relative_to(root))
        file=obj('file/'+rel,'isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = '+quote(rel)+'; sourceTree = "<group>";')
        if file not in refs: refs.append(file)
        sources.append(obj(name+'/build/'+rel,'isa = PBXBuildFile; fileRef = '+file+';'))
    product=obj(name+'/product','isa = PBXFileReference; explicitFileType = '+('wrapper.application' if ext=='app' else 'wrapper.app-extension')+'; path = '+name+'.'+ext+'; sourceTree = BUILT_PRODUCTS_DIR;')
    products.append(product)
    phase=obj(name+'/sources','isa = PBXSourcesBuildPhase; buildActionMask = 2147483647; files = '+arr(sources)+'; runOnlyForDeploymentPostprocessing = 0;')
    shell=obj(name+'/rust','isa = PBXShellScriptBuildPhase; buildActionMask = 2147483647; files = (); inputPaths = (); outputPaths = (); shellPath = /bin/sh; shellScript = '+quote('"$SRCROOT/build-rust.sh"')+'; alwaysOutOfDate = 1; runOnlyForDeploymentPostprocessing = 0;')
    phases=[shell,phase]; deps=[]
    if ext=='app':
        asset='Forwiroh/Assets.xcassets'
        assetfile=obj('file/'+asset,'isa = PBXFileReference; lastKnownFileType = folder.assetcatalog; path = '+quote(asset)+'; sourceTree = "<group>";')
        refs.append(assetfile)
        assetbuild=obj(name+'/build/'+asset,'isa = PBXBuildFile; fileRef = '+assetfile+';')
        phases.append(obj(name+'/resources','isa = PBXResourcesBuildPhase; buildActionMask = 2147483647; files = '+arr([assetbuild])+'; runOnlyForDeploymentPostprocessing = 0;'))
        copyfile=obj('embed/build','isa = PBXBuildFile; fileRef = '+ident('PacketTunnel/product')+'; settings = { ATTRIBUTES = (RemoveHeadersOnCopy,); };')
        phases.append(obj('embed','isa = PBXCopyFilesBuildPhase; buildActionMask = 2147483647; dstPath = ""; dstSubfolderSpec = 13; files = '+arr([copyfile])+'; name = "Embed App Extensions"; runOnlyForDeploymentPostprocessing = 0;'))
        proxy=obj('extension/proxy','isa = PBXContainerItemProxy; containerPortal = '+ident('project')+'; proxyType = 1; remoteGlobalIDString = '+ident('PacketTunnel/target')+'; remoteInfo = PacketTunnel;')
        deps.append(obj('extension/dep','isa = PBXTargetDependency; target = '+ident('PacketTunnel/target')+'; targetProxy = '+proxy+';'))
    settings={'PRODUCT_NAME':name,'PRODUCT_BUNDLE_IDENTIFIER':'org.radare.forwiroh'+('.PacketTunnel' if ext=='appex' else ''),'INFOPLIST_FILE':name+'/Info.plist','CODE_SIGN_ENTITLEMENTS':name+'/'+name+'.entitlements','CODE_SIGN_STYLE':'Automatic','SWIFT_VERSION':'6.0','IPHONEOS_DEPLOYMENT_TARGET':'17.0','SDKROOT':'iphoneos','SUPPORTED_PLATFORMS':'iphoneos iphonesimulator','TARGETED_DEVICE_FAMILY':'1,2','SWIFT_OBJC_BRIDGING_HEADER':'Shared/forwiroh.h','OTHER_LDFLAGS':'$(inherited) "$(DERIVED_FILE_DIR)/libforwiroh.a" -framework Security -framework SystemConfiguration -framework Network -framework CoreFoundation -lresolv -liconv','ENABLE_USER_SCRIPT_SANDBOXING':'NO','ONLY_ACTIVE_ARCH':'YES','FORWIROH_APP_GROUP':'group.org.radare.forwiroh','FORWIROH_KEYCHAIN_GROUP':'org.radare.forwiroh.shared','LD_RUNPATH_SEARCH_PATHS':'$(inherited) @executable_path/Frameworks @executable_path/../../Frameworks','GENERATE_INFOPLIST_FILE':'NO'}
    if ext=='appex': settings.update(APPLICATION_EXTENSION_API_ONLY='YES',SKIP_INSTALL='YES')
    else: settings.update(SKIP_INSTALL='NO',ASSETCATALOG_COMPILER_APPICON_NAME='AppIcon')
    target=obj(name+'/target','isa = PBXNativeTarget; buildConfigurationList = '+configs(name,settings)+'; buildPhases = '+arr(phases)+'; buildRules = (); dependencies = '+arr(deps)+'; name = '+name+'; productName = '+name+'; productReference = '+product+'; productType = '+quote('com.apple.product-type.application' if ext=='app' else 'com.apple.product-type.app-extension')+';')
    targets[name]=target
prodgroup=obj('products','isa = PBXGroup; children = '+arr(products)+'; name = Products; sourceTree = "<group>";')
group=obj('main','isa = PBXGroup; children = '+arr(refs+[prodgroup])+'; sourceTree = "<group>";')
project=obj('project','isa = PBXProject; attributes = { LastUpgradeCheck = 1600; }; buildConfigurationList = '+configs('project',{'CLANG_ENABLE_MODULES':'YES','SWIFT_STRICT_CONCURRENCY':'complete'})+'; compatibilityVersion = "Xcode 14.0"; developmentRegion = en; hasScannedForEncodings = 0; knownRegions = (en, Base,); mainGroup = '+group+'; productRefGroup = '+prodgroup+'; projectDirPath = ""; projectRoot = ""; targets = '+arr(list(targets.values()))+';')
projectdir=root/'Forwiroh.xcodeproj'; projectdir.mkdir(exist_ok=True)
(projectdir/'project.pbxproj').write_text('// !$*UTF8*$!\n{archiveVersion = 1; classes = {}; objectVersion = 56; objects = {\n'+''.join(i+' = {'+body+'};\n' for i,body in objects.items())+'}; rootObject = '+project+'; }\n')
scheme=projectdir/'xcshareddata/xcschemes';scheme.mkdir(parents=True,exist_ok=True)
(scheme/'Forwiroh.xcscheme').write_text('''<?xml version="1.0" encoding="UTF-8"?>
<Scheme LastUpgradeVersion="1600" version="1.3"><BuildAction parallelizeBuildables="YES" buildImplicitDependencies="YES"><BuildActionEntries><BuildActionEntry buildForTesting="YES" buildForRunning="YES" buildForProfiling="YES" buildForArchiving="YES" buildForAnalyzing="YES"><BuildableReference BuildableIdentifier="primary" BlueprintIdentifier="'''+targets['Forwiroh']+'''" BuildableName="Forwiroh.app" BlueprintName="Forwiroh" ReferencedContainer="container:Forwiroh.xcodeproj"/></BuildActionEntry></BuildActionEntries></BuildAction><LaunchAction buildConfiguration="Debug" selectedDebuggerIdentifier="Xcode.DebuggerFoundation.Debugger.LLDB" selectedLauncherIdentifier="Xcode.IDEFoundation.Launcher.LLDB" launchStyle="0" useCustomWorkingDirectory="NO" ignoresPersistentStateOnLaunch="NO" debugDocumentVersioning="YES" debugServiceExtension="internal" allowLocationSimulation="YES"><BuildableProductRunnable runnableDebuggingMode="0"><BuildableReference BuildableIdentifier="primary" BlueprintIdentifier="'''+targets['Forwiroh']+'''" BuildableName="Forwiroh.app" BlueprintName="Forwiroh" ReferencedContainer="container:Forwiroh.xcodeproj"/></BuildableProductRunnable></LaunchAction><ArchiveAction buildConfiguration="Release" revealArchiveInOrganizer="YES"/></Scheme>''')
