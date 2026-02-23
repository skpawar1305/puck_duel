# THIS FILE IS AUTO-GENERATED. DO NOT MODIFY!!

# Copyright 2020-2023 Tauri Programme within The Commons Conservancy
# SPDX-License-Identifier: Apache-2.0
# SPDX-License-Identifier: MIT

-keep class com.dano.puckduel.* {
  native <methods>;
}

-keep class com.dano.puckduel.WryActivity {
  public <init>(...);

  void setWebView(com.dano.puckduel.RustWebView);
  java.lang.Class getAppClass(...);
  java.lang.String getVersion();
}

-keep class com.dano.puckduel.Ipc {
  public <init>(...);

  @android.webkit.JavascriptInterface public <methods>;
}

-keep class com.dano.puckduel.RustWebView {
  public <init>(...);

  void loadUrlMainThread(...);
  void loadHTMLMainThread(...);
  void evalScript(...);
}

-keep class com.dano.puckduel.RustWebChromeClient,com.dano.puckduel.RustWebViewClient {
  public <init>(...);
}
