package fr.hypolia.lobby.entities;

import org.bukkit.ChatColor;
import org.bukkit.Location;
import org.bukkit.entity.Player;

public class WelcomeNPC extends AbstractNPC {
  public WelcomeNPC(Location location) {
    super(ChatColor.GOLD + "Welcome NPC", location);
  }

  @Override
  public void handleClick(Player player) {
    player.sendMessage("Bienvenue sur Hypolia!");
  }
}
