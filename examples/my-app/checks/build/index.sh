#!/bin/sh
exec xcodebuild -project My_App.xcodeproj -scheme My_App -destination platform=macOS CODE_SIGNING_ALLOWED=NO build
