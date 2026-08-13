package com.vibeeasytier.a14.service;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.net.VpnService;

import com.vibeeasytier.a14.storage.AppPreferences;
import com.vibeeasytier.a14.storage.SecureProfileStore;

public final class BootReceiver extends BroadcastReceiver {
    @Override
    public void onReceive(Context context, Intent intent) {
        String action = intent == null ? "" : intent.getAction();
        if (!Intent.ACTION_BOOT_COMPLETED.equals(action)
                && !Intent.ACTION_MY_PACKAGE_REPLACED.equals(action)) {
            return;
        }
        AppPreferences preferences = new AppPreferences(context);
        if (!preferences.autoConnect() || !new SecureProfileStore(context).exists()) {
            return;
        }
        if (VpnService.prepare(context) != null) {
            preferences.writeStatus("FAILED", "需要重新授予系统 VPN 权限", 0, 0, 0, 0, 0, 0);
            return;
        }
        Intent service = EasyTierVpnService.connectIntent(context, false);
        context.startForegroundService(service);
    }
}
