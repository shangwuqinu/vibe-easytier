package com.vibeeasytier.a14.service;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.net.ConnectivityManager;
import android.net.Network;
import android.net.NetworkCapabilities;
import android.net.VpnService;
import android.os.ParcelFileDescriptor;

import com.easytier.jni.EasyTierJNI;
import com.vibeeasytier.a14.MainActivity;
import com.vibeeasytier.a14.R;
import com.vibeeasytier.a14.config.TomlProfileCodec;
import com.vibeeasytier.a14.core.Ipv4Cidr;
import com.vibeeasytier.a14.core.NetworkSnapshot;
import com.vibeeasytier.a14.core.NetworkStatusParser;
import com.vibeeasytier.a14.core.RetryPolicy;
import com.vibeeasytier.a14.model.Profile;
import com.vibeeasytier.a14.model.ProfileValidator;
import com.vibeeasytier.a14.storage.AppPreferences;
import com.vibeeasytier.a14.storage.LogStore;
import com.vibeeasytier.a14.storage.SecureProfileStore;

import java.util.List;
import java.util.concurrent.ScheduledThreadPoolExecutor;
import java.util.concurrent.ThreadLocalRandom;
import java.util.concurrent.TimeUnit;

public final class EasyTierVpnService extends VpnService {
    public static final String ACTION_CONNECT = "com.vibeeasytier.a14.CONNECT";
    public static final String ACTION_RELOAD = "com.vibeeasytier.a14.RELOAD";
    public static final String ACTION_STOP = "com.vibeeasytier.a14.STOP";
    public static final String ACTION_STATUS = "com.vibeeasytier.a14.STATUS";
    public static final String INTERNAL_STATUS_PERMISSION = "com.vibeeasytier.a14.permission.INTERNAL_STATUS";
    public static final String EXTRA_NODES = "nodes";
    private static final String EXTRA_USER_INITIATED = "user_initiated";
    private static final String CHANNEL_ID = "vibe_vpn";
    private static final int NOTIFICATION_ID = 11010;
    private static final long NO_PEER_RESTART_MS = TimeUnit.MINUTES.toMillis(10);
    private static final long NO_PEER_RESTART_LIMIT_MS = TimeUnit.MINUTES.toMillis(15);

    private final ScheduledThreadPoolExecutor worker = new ScheduledThreadPoolExecutor(1);
    private SecureProfileStore profileStore;
    private AppPreferences preferences;
    private LogStore logs;
    private ConnectivityManager connectivityManager;
    private ConnectivityManager.NetworkCallback networkCallback;
    private ParcelFileDescriptor tunInterface;
    private Profile currentProfile;
    private volatile boolean coreRunning;
    private volatile boolean stopping;
    private int retryAttempt;
    private long connectionStartedAt;
    private long lastNoPeerRestartAt;
    private long lastSuccess;
    private List<String> nodes = List.of();

    public static Intent connectIntent(Context context, boolean userInitiated) {
        return new Intent(context, EasyTierVpnService.class)
                .setAction(ACTION_CONNECT)
                .putExtra(EXTRA_USER_INITIATED, userInitiated);
    }

    public static Intent stopIntent(Context context) {
        return new Intent(context, EasyTierVpnService.class).setAction(ACTION_STOP);
    }

    @Override
    public void onCreate() {
        super.onCreate();
        profileStore = new SecureProfileStore(this);
        preferences = new AppPreferences(this);
        logs = new LogStore(this);
        lastSuccess = preferences.readStatus().lastSuccess();
        createNotificationChannel();
        connectivityManager = getSystemService(ConnectivityManager.class);
        networkCallback = new ConnectivityManager.NetworkCallback() {
            @Override
            public void onAvailable(Network network) {
                worker.execute(() -> {
                    if (!stopping && !coreRunning && shouldRun()) {
                        logs.append("系统网络已恢复，准备重新连接");
                        scheduleConnect(0);
                    }
                });
            }

            @Override
            public void onLost(Network network) {
                worker.execute(() -> {
                    if (coreRunning) {
                        updateState("RECOVERING", "系统网络已断开，等待恢复", 0, 0);
                    }
                });
            }
        };
        connectivityManager.registerDefaultNetworkCallback(networkCallback);
        worker.scheduleWithFixedDelay(this::monitorCore, 3, 3, TimeUnit.SECONDS);
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        String action = intent == null ? ACTION_CONNECT : intent.getAction();
        if (ACTION_STOP.equals(action)) {
            if (isAlwaysOn()) {
                preferences.setAlwaysOn(true);
                updateState("CONNECTED", "Always-on VPN 已启用，请在系统设置中断开", nodes.size(), 0);
                return START_STICKY;
            }
            preferences.setAutoConnect(false);
            stopping = true;
            worker.execute(() -> {
                stopRuntime();
                updateState("DISCONNECTED", "已手动断开，自动连接已关闭", 0, 0);
                logs.append("已手动断开私有网络");
                stopForeground(STOP_FOREGROUND_REMOVE);
                stopSelf();
            });
            return START_NOT_STICKY;
        }

        startForegroundCompat(buildNotification("正在准备私有网络"));
        stopping = false;
        if (ACTION_RELOAD.equals(action)) {
            worker.execute(() -> {
                stopRuntime();
                scheduleConnect(0);
            });
            return START_STICKY;
        }
        boolean userInitiated = intent != null && intent.getBooleanExtra(EXTRA_USER_INITIATED, false);
        if (!userInitiated && !shouldRun()) {
            updateState("DISCONNECTED", "自动连接未启用", 0, 0);
            stopForeground(STOP_FOREGROUND_REMOVE);
            stopSelf();
            return START_NOT_STICKY;
        }
        worker.execute(() -> scheduleConnect(0));
        return START_STICKY;
    }

    @Override
    public void onRevoke() {
        preferences.setAutoConnect(false);
        stopping = true;
        worker.execute(() -> {
            stopRuntime();
            updateState("FAILED", "系统已撤销 VPN 权限", 0, 0);
            stopSelf();
        });
        super.onRevoke();
    }

    @Override
    public void onDestroy() {
        stopping = true;
        if (connectivityManager != null && networkCallback != null) {
            try {
                connectivityManager.unregisterNetworkCallback(networkCallback);
            } catch (Exception ignored) {
                // Already unregistered by the platform.
            }
        }
        stopRuntime();
        worker.shutdownNow();
        super.onDestroy();
    }

    private boolean shouldRun() {
        return preferences.autoConnect() || isAlwaysOn();
    }

    private void scheduleConnect(long delayMs) {
        if (stopping || coreRunning) {
            return;
        }
        worker.schedule(this::connect, Math.max(0, delayMs), TimeUnit.MILLISECONDS);
    }

    private synchronized void connect() {
        if (stopping || coreRunning) {
            return;
        }
        if (!hasUsableNetwork()) {
            retry("当前没有可用的系统网络");
            return;
        }
        if (VpnService.prepare(this) != null) {
            failWithoutRetry("需要先在应用内授予系统 VPN 权限");
            return;
        }
        if (!profileStore.exists()) {
            failWithoutRetry("尚未保存私有网络档案");
            return;
        }
        if (!EasyTierJNI.isAvailable()) {
            failWithoutRetry("内置 EasyTier Core 2.6.4 组件未暂存");
            return;
        }

        updateState("CONNECTING", "正在启动 EasyTier Core", 0, 0);
        try {
            stopRuntime();
            Profile profile = profileStore.load();
            currentProfile = profile;
            ProfileValidator.validate(profile);
            String config = TomlProfileCodec.render(profile);
            int parsed = EasyTierJNI.parseConfig(config);
            if (parsed != 0) {
                throw new IllegalStateException(nativeError("Core 拒绝配置"));
            }
            int started = EasyTierJNI.runNetworkInstance(config);
            if (started != 0) {
                throw new IllegalStateException(nativeError("Core 启动失败"));
            }
            EasyTierJNI.retainNetworkInstance(new String[]{profile.instanceName()});
            ParcelFileDescriptor established = establishTun(profile);
            int tunResult = EasyTierJNI.setTunFd(profile.instanceName(), established.getFd());
            if (tunResult != 0) {
                established.close();
                throw new IllegalStateException(nativeError("Core 无法接管 TUN 接口"));
            }
            currentProfile = profile;
            tunInterface = established;
            coreRunning = true;
            retryAttempt = 0;
            connectionStartedAt = System.currentTimeMillis();
            nodes = List.of();
            logs.append("EasyTier Core 已启动，正在等待远端节点");
            updateState("CONNECTING", "Core 已运行，正在等待远端节点", 0, 0);
        } catch (Exception | LinkageError error) {
            String message = safeError(error);
            stopRuntime();
            retry(message);
        }
    }

    private ParcelFileDescriptor establishTun(Profile profile) throws Exception {
        Ipv4Cidr cidr = Ipv4Cidr.parse(profile.ipv4Cidr());
        Builder builder = new Builder()
                .setSession("Vibe EasyTier · " + profile.profileName())
                .addAddress(cidr.address(), cidr.prefix())
                .addRoute(cidr.networkAddress(), cidr.prefix())
                .setConfigureIntent(PendingIntent.getActivity(
                        this,
                        0,
                        new Intent(this, MainActivity.class),
                        PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT));
        Object mtuValue = profile.flags().get("mtu");
        if (mtuValue instanceof Number number) {
            builder.setMtu(Math.max(576, Math.min(9000, number.intValue())));
        }
        builder.addDisallowedApplication(getPackageName());
        ParcelFileDescriptor descriptor = builder.establish();
        if (descriptor == null) {
            throw new IllegalStateException("系统未能创建 VPN 接口");
        }
        return descriptor;
    }

    private synchronized void monitorCore() {
        if (!coreRunning || currentProfile == null || stopping) {
            return;
        }
        try {
            NetworkSnapshot snapshot = NetworkStatusParser.parse(
                    EasyTierJNI.collectNetworkInfos(), currentProfile.instanceName());
            if (!snapshot.running()) {
                throw new IllegalStateException(snapshot.error().isBlank()
                        ? "Core 网络实例已停止" : snapshot.error());
            }
            nodes = snapshot.nodes();
            if (snapshot.peerCount() > 0) {
                if (lastSuccess == 0) {
                    logs.append("私有网络连接成功");
                }
                lastSuccess = System.currentTimeMillis();
                updateState("CONNECTED", "已连接私有网络", snapshot.peerCount(), 0);
                return;
            }
            updateState("CONNECTING", "Core 正常，尚无远端节点", 0, 0);
            long now = System.currentTimeMillis();
            if (now - connectionStartedAt >= NO_PEER_RESTART_MS
                    && now - lastNoPeerRestartAt >= NO_PEER_RESTART_LIMIT_MS) {
                lastNoPeerRestartAt = now;
                logs.append("持续无远端节点，受控重启 Core");
                stopRuntime();
                retry("持续无远端节点");
            }
        } catch (Exception | LinkageError error) {
            String message = safeError(error);
            logs.append("Core 健康检查失败：" + message);
            stopRuntime();
            retry(message);
        }
    }

    private boolean hasUsableNetwork() {
        Network active = connectivityManager.getActiveNetwork();
        NetworkCapabilities capabilities = connectivityManager.getNetworkCapabilities(active);
        return capabilities != null
                && capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET);
    }

    private void retry(String reason) {
        if (stopping || !shouldRun()) {
            return;
        }
        long base = RetryPolicy.delayMs(retryAttempt, 0);
        long jitter = ThreadLocalRandom.current().nextLong(Math.max(1, base / 5));
        long delay = RetryPolicy.delayMs(retryAttempt, jitter);
        retryAttempt++;
        long retryAt = System.currentTimeMillis() + delay;
        updateState("RECOVERING", reason, 0, retryAt);
        logs.append("连接失败，将自动重试：" + reason);
        scheduleConnect(delay);
    }

    private void failWithoutRetry(String reason) {
        updateState("FAILED", reason, 0, 0);
        logs.append(reason);
        stopForeground(STOP_FOREGROUND_REMOVE);
        stopSelf();
    }

    private synchronized void stopRuntime() {
        coreRunning = false;
        currentProfile = null;
        nodes = List.of();
        if (tunInterface != null) {
            try {
                tunInterface.close();
            } catch (Exception ignored) {
                // Closing an already-revoked VPN descriptor is harmless.
            }
            tunInterface = null;
        }
        if (EasyTierJNI.isAvailable()) {
            try {
                EasyTierJNI.retainNetworkInstance(new String[0]);
            } catch (Exception | LinkageError ignored) {
                // The health state already records the actionable failure.
            }
        }
    }

    private String nativeError(String fallback) {
        try {
            String error = EasyTierJNI.getLastError();
            return error == null || error.isBlank() ? fallback : error;
        } catch (Exception | LinkageError ignored) {
            return fallback;
        }
    }

    private String safeError(Throwable error) {
        String message = error.getMessage();
        if (message == null || message.isBlank()) {
            message = error.getClass().getSimpleName();
        }
        if (currentProfile != null && !currentProfile.networkSecret().isEmpty()) {
            message = message.replace(currentProfile.networkSecret(), "[已隐藏]");
        }
        return message.replace('\n', ' ').replace('\r', ' ').trim();
    }

    private void updateState(String state, String detail, int peerCount, long retryAt) {
        preferences.setAlwaysOn(isAlwaysOn());
        preferences.writeStatus(state, detail, peerCount, lastSuccess, retryAt);
        Intent broadcast = new Intent(ACTION_STATUS)
                .setPackage(getPackageName())
                .putStringArrayListExtra(EXTRA_NODES, new java.util.ArrayList<>(nodes));
        sendBroadcast(broadcast, INTERNAL_STATUS_PERMISSION);
        NotificationManager manager = getSystemService(NotificationManager.class);
        manager.notify(NOTIFICATION_ID, buildNotification(notificationText(state, detail, peerCount)));
    }

    private String notificationText(String state, String detail, int peerCount) {
        if ("CONNECTED".equals(state)) {
            return "已连接，远端节点 " + peerCount + " 个";
        }
        return detail;
    }

    private Notification buildNotification(String content) {
        PendingIntent open = PendingIntent.getActivity(
                this,
                0,
                new Intent(this, MainActivity.class),
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        PendingIntent stop = PendingIntent.getService(
                this,
                1,
                stopIntent(this),
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        return new Notification.Builder(this, CHANNEL_ID)
                .setSmallIcon(android.R.drawable.stat_sys_upload)
                .setContentTitle("Vibe EasyTier")
                .setContentText(content)
                .setContentIntent(open)
                .setOngoing(true)
                .setOnlyAlertOnce(true)
                .addAction(new Notification.Action.Builder(null, "断开", stop).build())
                .build();
    }

    private void createNotificationChannel() {
        NotificationChannel channel = new NotificationChannel(
                CHANNEL_ID,
                getString(R.string.vpn_channel_name),
                NotificationManager.IMPORTANCE_LOW);
        channel.setDescription(getString(R.string.vpn_channel_description));
        getSystemService(NotificationManager.class).createNotificationChannel(channel);
    }

    private void startForegroundCompat(Notification notification) {
        startForeground(
                NOTIFICATION_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE);
    }
}
