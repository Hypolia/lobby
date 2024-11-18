package fr.hypolia.lobby;

import fr.hypolia.lobby.utils.ScoreboardBuilder;
import org.bukkit.entity.Player;

public class LobbyScoreboard {
  public static void setScoreboard(Player player) {
    new ScoreboardBuilder("§6§lHYPOLIA")
        .setLine(15, "§711/16/24 §8L12F")
        .setLine(14, "§7")
        .setLine(13, "§fRank: §cAdministrateur")
        .setLine(12, "§fSuccès: §e0")
        .setLine(11, "§fNiveau Hypolia: §31")
        .setLine(10, "§7 ")
        .setLine(9, "§fLobby: §a1")
        .setLine(8, "§fJoueurs: §a1")
        .setLine(7, "§f  ")
        .setLine(6, "§6www.hypolia.fr")
        .applyToPlayer(player);
  }
}
