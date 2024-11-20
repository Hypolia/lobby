package fr.hypolia.lobby.events;

import fr.hypolia.lobby.LobbyItems;
import fr.hypolia.lobby.LobbyScoreboard;
import fr.hypolia.lobby.PlayerApiClient;
import fr.hypolia.lobby.inventories.CountItem;
import fr.hypolia.lobby.utils.ItemBuilder;
import org.bukkit.Location;
import org.bukkit.Material;
import org.bukkit.Sound;
import org.bukkit.World;
import org.bukkit.entity.Player;
import org.bukkit.event.EventHandler;
import org.bukkit.event.Listener;
import org.bukkit.event.entity.EntityDamageEvent;
import org.bukkit.event.entity.EntityTargetEvent;
import org.bukkit.event.player.PlayerInteractEvent;
import org.bukkit.event.player.PlayerJoinEvent;
import org.bukkit.inventory.EquipmentSlot;
import org.bukkit.inventory.ItemStack;
import xyz.xenondevs.invui.gui.Gui;
import xyz.xenondevs.invui.item.impl.SimpleItem;
import xyz.xenondevs.invui.window.Window;

import java.io.IOException;


public class LobbyEvents implements Listener {
  //private final PlayerApiClient playerApiClient;

//  public LobbyEvents(PlayerApiClient playerApiClient) {
//    this.playerApiClient = playerApiClient;
//  }

  @EventHandler
  public void onPlayerJoin(PlayerJoinEvent event) throws IOException {
    Player player = event.getPlayer();

    event.joinMessage(null);

    World world = player.getWorld();

    //playerApiClient.getPlayerInfo(player.getUniqueId());

    Location teleportLocation = new Location(world, 16.5, 12, 0.5, 90, 0);
    player.teleport(teleportLocation);

    LobbyItems.giveLobbyItems(player);
    LobbyScoreboard.setScoreboard(event.getPlayer());

    player.sendMessage("§aBienvenue sur le serveur !");
  }

  @EventHandler
  public void onEntityDamage(EntityDamageEvent event) {
    event.setCancelled(true);
  }

  @EventHandler
  public void onEntityTarget(EntityTargetEvent event) {
    event.setCancelled(true);
  }

  @EventHandler
  public void onExplosion(EntityDamageEvent event) {
    if (event.getCause() == EntityDamageEvent.DamageCause.ENTITY_EXPLOSION ||
        event.getCause() == EntityDamageEvent.DamageCause.BLOCK_EXPLOSION
    ) {
      event.setCancelled(true);
    }
  }

  @EventHandler
  public void onPlayerInteract(PlayerInteractEvent event) {
    Player player = event.getPlayer();
    ItemStack item = event.getItem();

    if (item != null && item.getType() == Material.COMPASS) {
      if (event.getHand() == EquipmentSlot.HAND) {
        // Jouer un son lorsque le menu s'ouvre
        player.playSound(player.getLocation(), Sound.BLOCK_NOTE_BLOCK_PLING, 1.0f, 1.0f);
        ItemStack item1 = new ItemBuilder(Material.BLACK_STAINED_GLASS_PANE)
            .setName("§b")
            .setUnbreakable(true)
            .build();

        CountItem countItem = new CountItem();
        Gui gui = Gui.normal()
            .setStructure(
                "# # # # # # # # #",
                "# . . . . . . . #",
                "# . . . . . . . #",
                "# # # # # # # # #")
            .addIngredient('#', new SimpleItem(item1))
            .build();

        gui.setItem(10, countItem);

        Window window = Window.single()
            .setViewer(player)
            .setTitle("InvUI")
            .setGui(gui)
            .build();

        window.open();

      }
    }

  }
}
