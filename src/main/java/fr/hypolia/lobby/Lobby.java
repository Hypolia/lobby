package fr.hypolia.lobby;

import fr.hypolia.lobby.commands.SpawnNPC;
import fr.hypolia.lobby.entities.WelcomeNPC;
import fr.hypolia.lobby.events.LobbyEvents;
import fr.hypolia.lobby.managers.NPCManager;
import fr.hypolia.lobby.services.KeycloakService;
import org.bukkit.Bukkit;
import org.bukkit.Location;
import org.bukkit.World;
import org.bukkit.entity.Entity;
import org.bukkit.entity.EntityType;
import org.bukkit.entity.Villager;
import org.bukkit.plugin.java.JavaPlugin;

public final class Lobby extends JavaPlugin {
  private PlayerApiClient playerApiClient;
  private NPCManager npcManager;

  private static Lobby instance;

  public static Lobby getInstance() {
    return instance;
  }

  public NPCManager getNpcManager() {
    return npcManager;
  }

  @Override
  public void onEnable() {
    instance = this;
    getLogger().info("Lobby plugin enabled");

    KeycloakService keycloakService = new KeycloakService(
        "http://hypolia-keycloak-1:8080",
        "hypolia",
        "lobby",
        "af9gDHh6HabWr1yNvW7JOgXhPZVY1NwB"
    );

    npcManager = new NPCManager();
    /*

    playerApiClient = new PlayerApiClient("http://localhost:3333", keycloakService);*/
    cleanUpLobby();

    spawnNCP();

    getServer().getPluginManager().registerEvents(new LobbyEvents(), this);

  }

  @Override
  public void onDisable() {
    getLogger().info("Lobby plugin disabled");
    cleanUpLobby();
  }

  private void cleanUpLobby () {
    World world = getServer().getWorld("world");

    if (world == null) {
      getLogger().warning("Le monde 'world' n'existe pas. Impossible de faire spawn le PNJ.");
      return;
    }


    getLogger().info("Nettoyage du lobby...");

    Location lobbyCenter = new Location(world, 0, 12, 0); // Coordonnées approximatives du lobby
    double cleanupRadius = 200.0; // Rayon du nettoyage autour du centre

    for (Entity entity : world.getNearbyEntities(lobbyCenter, cleanupRadius, cleanupRadius, cleanupRadius)) {
      if (entity.getType() == EntityType.VILLAGER) {
        getLogger().info("Suppression du PNJ " + entity.getUniqueId());
        entity.remove();
      }
    }

  }

  private void spawnNCP() {
    World world = getServer().getWorld("world");

    if (world == null) {
      getLogger().warning("Le monde 'world' n'existe pas. Impossible de faire spawn le PNJ.");
      return;
    }

    Location location = new Location(world, -24.5, 12, 8.5, -90, 0);
    double searchRadius = 1.0;

    WelcomeNPC welcomeNPC = new WelcomeNPC(location);
    npcManager.registerNPC(welcomeNPC);

    getLogger().info("PNJ 'Guide du Lobby' spawn à " + location.getX() + ", " + location.getY() + ", " + location.getZ());
  }
}
