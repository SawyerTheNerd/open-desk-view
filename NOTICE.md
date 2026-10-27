# Notice & disclaimer

**OpenDeskView is an independent, open-source project. It is not affiliated with, authorized by,
endorsed by, or sponsored by datronicsoft or the spacedesk product in any way.**

## Trademarks

"spacedesk" and "datronicsoft" are trademarks of their respective owners. They are used in this
project **only nominatively** — i.e., to describe the fact that OpenDeskView is *compatible with* the
spacedesk display-streaming protocol — and never as the name of this software or to imply any
connection with, or approval by, the trademark holder. OpenDeskView is the name of this project.

## What this project is (and is not)

- It is a **clean-room, interoperability** implementation of the spacedesk network protocol: an
  independent viewer (client) and an independent server, both written from scratch. The viewer can
  connect to OpenDeskView's own server or to a spacedesk Server **that you install and run yourself**.
- The protocol description in [`docs/PROTOCOL.md`](docs/PROTOCOL.md) was reconstructed from observed,
  interoperable behavior and public clean-room documentation. This repository contains **no** source
  code, binaries, decompiled output, assets, or other proprietary material from datronicsoft or the
  spacedesk product, and none may be added.
- This project does not distribute the spacedesk Server or circumvent any technical protection
  measure. If you choose to use the spacedesk Server, obtain and run it separately under its own license.

## Third-party components (not distributed with OpenDeskView)

- **Virtual Display Driver** ([VirtualDrivers/Virtual-Display-Driver](https://github.com/VirtualDrivers/Virtual-Display-Driver),
  MIT) provides the virtual monitor for extended mode. Install it from that project.
- **OpenH264** (Cisco). The Windows server and desktop viewer can use Cisco's prebuilt
  `openh264-2.6.0-win64.dll` for H.264. It is **not** bundled: download it yourself from Cisco
  (see the README). H.264 is optional — `spacedesk-server --no-h264` turns it off, and the desktop
  viewer only uses it with `--h264`. The Android app uses the device's own decoder, not OpenH264.

  OpenH264 Video Codec provided by Cisco Systems, Inc.

  Cisco's binary license, reproduced as it requires:

  ```text
  -------------------------------------------------------
  About The Cisco-Provided Binary of OpenH264 Video Codec
  -------------------------------------------------------

  Cisco provides this program under the terms of the BSD license.

  Additionally, this binary is licensed under Cisco's AVC/H.264 Patent Portfolio License from MPEG LA, at no cost to you, provided that the requirements and conditions shown below in the AVC/H.264 Patent Portfolio sections are met.

  As with all AVC/H.264 codecs, you may also obtain your own patent license from MPEG LA or from the individual patent owners, or proceed at your own risk.  Your rights from Cisco under the BSD license are not affected by this choice.

  For more information on the OpenH264 binary licensing, please see the OpenH264 FAQ found at http://www.openh264.org/faq.html#binary

  A corresponding source code to this binary program is available under the same BSD terms, which can be found at http://www.openh264.org

  -----------
  BSD License
  -----------

  Copyright © 2014 Cisco Systems, Inc.

  All rights reserved.

  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the following conditions are met:

  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following disclaimer.

  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the following disclaimer in the documentation and/or other materials provided with the distribution.

  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED.  IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

  -----------------------------------------
  AVC/H.264 Patent Portfolio License Notice
  -----------------------------------------

  The binary form of this Software is distributed by Cisco under the AVC/H.264 Patent Portfolio License from MPEG LA, and is subject to the following requirements, which may or may not be applicable to your use of this software:

  THIS PRODUCT IS LICENSED UNDER THE AVC PATENT PORTFOLIO LICENSE FOR THE PERSONAL USE OF A CONSUMER OR OTHER USES IN WHICH IT DOES NOT RECEIVE REMUNERATION TO (i) ENCODE VIDEO IN COMPLIANCE WITH THE AVC STANDARD ("AVC VIDEO") AND/OR (ii) DECODE AVC VIDEO THAT WAS ENCODED BY A CONSUMER ENGAGED IN A PERSONAL ACTIVITY AND/OR WAS OBTAINED FROM A VIDEO PROVIDER LICENSED TO PROVIDE AVC VIDEO.  NO LICENSE IS GRANTED OR SHALL BE IMPLIED FOR ANY OTHER USE.  ADDITIONAL INFORMATION MAY BE OBTAINED FROM MPEG LA, L.L.C. SEE HTTP://WWW.MPEGLA.COM

  Accordingly, please be advised that content providers and broadcasters using AVC/H.264 in their service may be required to obtain a separate use license from MPEG LA, referred to as "(b) sublicenses" in the SUMMARY OF AVC/H.264 LICENSE TERMS from MPEG LA found at http://www.openh264.org/mpegla

  ---------------------------------------------
  AVC/H.264 Patent Portfolio License Conditions
  ---------------------------------------------

  In addition, the Cisco-provided binary of this Software is licensed under Cisco's license from MPEG LA only if the following conditions are met:

  1. The Cisco-provided binary is separately downloaded to an end user's device, and not integrated into or combined with third party software prior to being downloaded to the end user's device;

  2. The end user must have the ability to control (e.g., to enable, disable, or re-enable) the use of the Cisco-provided binary;

  3. Third party software, in the location where end users can control the use of the Cisco-provided binary, must display the following text:

         "OpenH264 Video Codec provided by Cisco Systems, Inc."

  4.  Any third-party software that makes use of the Cisco-provided binary must reproduce all of the above text, as well as this last condition, in the EULA and/or in another location where licensing information is to be presented to the end user.

                            v1.0
  ```

## No warranty / your responsibility

This software is provided "AS IS", without warranty of any kind (see [`LICENSE`](LICENSE)). You are
responsible for ensuring your own use complies with all agreements you have entered into (including any
end-user license agreement accompanying the spacedesk Server, if you use it) and with the laws of your
jurisdiction.
Nothing here is legal advice.

## Contact

If you are a rights holder with a concern about this project, please open an issue and we will respond
promptly.
