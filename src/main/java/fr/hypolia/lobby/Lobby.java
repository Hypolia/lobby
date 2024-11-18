package fr.hypolia.lobby;

import fr.hypolia.lobby.commands.SpawnNPC;
import fr.hypolia.lobby.events.LobbyEvents;
import fr.hypolia.lobby.services.KeycloakService;
import org.bukkit.Location;
import org.bukkit.World;
import org.bukkit.entity.Entity;
import org.bukkit.entity.EntityType;
import org.bukkit.entity.Villager;
import org.bukkit.plugin.java.JavaPlugin;

public final class Lobby extends JavaPlugin {
  private PlayerApiClient playerApiClient;

  @Override
  public void onEnable() {
    getLogger().info("Lobby plugin enabled");

    KeycloakService keycloakService = new KeycloakService(
        "http://hypolia-keycloak-1:8080",
        "hypolia",
        "lobby",
        "af9gDHh6HabWr1yNvW7JOgXhPZVY1NwB"
    );


    /*

    playerApiClient = new PlayerApiClient("http://localhost:3333", keycloakService);*/
    cleanUpLobby();

    this.getCommand("createnpc").setExecutor(new SpawnNPC());
    getServer().getPluginManager().registerEvents(new LobbyEvents(), this);

    spawnNCP();
  }

  @Override
  public void onDisable() {
    getLogger().info("Lobby plugin disabled");
  }

  private void cleanUpLobby () {
    World world = getServer().getWorld("world");

    if (world == null) {
      getLogger().warning("Le monde 'world' n'existe pas. Impossible de faire spawn le PNJ.");
      return;
    }

    Location lobbyCenter = new Location(world, 0, 12, 0); // Coordonnées approximatives du lobby
    double cleanupRadius = 200.0; // Rayon du nettoyage autour du centre

    world.getNearbyEntities(lobbyCenter, cleanupRadius, cleanupRadius, cleanupRadius).stream()
      .filter(entity -> entity instanceof Villager)
      .forEach(Entity::remove);
  }

  private void spawnNCP() {
    World world = getServer().getWorld("world");

    if (world == null) {
      getLogger().warning("Le monde 'world' n'existe pas. Impossible de faire spawn le PNJ.");
      return;
    }

    Location location = new Location(world, -24.5, 12, 8.5, -90, 0);
    double searchRadius = 1.0;

    Villager npc = (Villager) world.spawnEntity(location, EntityType.VILLAGER);

    npc.setCustomName("§6Guide du Lobby");
    npc.setCustomNameVisible(true);
    npc.setAI(false); // Empêche les déplacements
    npc.setCollidable(false); // Empêche les interactions physiques avec les joueurs
    npc.setInvulnerable(true); // Empêche les dégâts

    getLogger().info("PNJ 'Guide du Lobby' spawn à " + location.getX() + ", " + location.getY() + ", " + location.getZ());
  }
}
